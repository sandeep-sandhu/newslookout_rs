// file: mod_summarize

use std::collections::HashMap;
use std::error::Error;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use config::Config;
use log::{debug, error, info};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, InvalidHeaderName};
use samvadsetu::llm::{LLMTextGenBuilder, LLMTextGenerator};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::{document, get_cfg, llm};
use samvadsetu::providers::{google, ollama, openai};
use crate::network::build_llm_api_client;
use crate::utils::{word_count};
use crate::get_plugin_cfg;
use crate::llm::{LLMParameters, TOKENS_PER_WORD, MIN_ACCEPTABLE_INPUT_CHARS};

pub const PLUGIN_NAME: &str = "mod_summarize";
pub const PUBLISHER_NAME: &str = "Text Summarization";

/// Executes this function of the module in the separate thread launched by the pipeline to
/// process documents received on channel rx and,
/// transmit the updated documents to tx.
///
/// # Arguments
///
/// * `tx`: Queue transmitter for the next thread
/// * `rx`: Queue receiver for this thread
/// * `config`: The application's configuration object
///
/// returns: ()
///
pub fn process_data(tx: Sender<document::Document>, rx: Receiver<document::Document>, app_config: &Config, api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>){

    info!("{}: Getting configuration specific to the module.", PLUGIN_NAME);
    let mut doc_counter: u32 = 0;

    let mut prompt_summary_part = get_cfg!("prompt_summary_part", app_config, "Summarize this text:\n");

    let mut prompt_summary_exec = get_cfg!("prompt_summary_exec", app_config, "Summarize this text:\n");

    let llm_service_name = get_plugin_cfg!(PLUGIN_NAME, "llm_service", app_config).unwrap_or_else(|| "ollama".to_string());

    let mut option_mutex = api_mutexes.get(llm_service_name.as_str());

    if let Some(mut llm_gen) = LLMTextGenBuilder::build_from_config(app_config, llm_service_name.as_str()) {

        if let Some(mut llm_api_mutex) = option_mutex {

            llm_gen.shared_lock = Option::from(Arc::clone(llm_api_mutex));

            // process each document received and return back to next handler:
            for mut doc in rx {
                update_doc(&mut llm_gen, &mut doc, prompt_summary_exec.clone(), prompt_summary_part.clone());

                //for each document processed, send it to next queue:
                match tx.send(doc) {
                    Result::Ok(_) => { doc_counter += 1; },
                    Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
                }
            }
            info!("{}: Completed processing {} documents.", PLUGIN_NAME, doc_counter);
        }

    } else {
        error!("Could not initialise LLM API. Exiting now.");
        panic!("Could not initialise LLM API. Stopping all execution now.");
    }
}

pub fn generate_text_using_llm(
    llm_gen: &mut LLMTextGenerator,
    input_context_prefix: String,
    user_prompt: String,
    input_context_suffix: String,
) -> String {

    llm_gen.user_prompt = user_prompt;
    if (input_context_prefix.len() + input_context_suffix.len()) > MIN_ACCEPTABLE_INPUT_CHARS {
        let llm_gen_result = llm_gen.generate_text(
            input_context_prefix.as_str(),
            input_context_suffix.as_str()
        );
        if let Ok(llm_api_response) = llm_gen_result {
            return llm_api_response.generated_text

        } else if let Err(e) = llm_gen_result {
            error!("When summarizing text: {}", e);
            return String::new()
        }
    }else{
        error!("Inadequate text given for processing: '{}' and '{}'", input_context_prefix, input_context_suffix);
    }
    String::new()
}

/// Generate the summary of this document's text.
/// Executes the text content with LLM prompt and returns back the generated content
/// first, get the word count from the input text + token and multiply this by tokens/per word
/// to get token count.
/// If this is higher than the available context length, i.e. llm_gen.num_context, then,
/// firstly - generate the output for the parts using the given parts prompt,
/// secondly - generate a summary of these summaries
/// Here, context prefix is not required, context suffix is the text of each part.
///
/// # Arguments
///
/// * `llm_gen`: The object that encapsulates the call to the LLM API service
/// * `raw_doc`: The document object with the text content to be summarised in fields "text"
///              and the text chunks are stored in the field "text_parts" -> "text"
/// * `prompt_summary_exec`: The summary prompt used to generate content for the entire text
/// * `prompt_summary_part`: The summary prompt for part by part summary
///
/// returns: ()
fn update_doc(
    llm_gen: &mut LLMTextGenerator,
    raw_doc: &mut document::Document,
    prompt_summary_exec: String,
    prompt_summary_part: String
) {
    let estimated_tokens = (word_count(raw_doc.text.as_str()) +
        word_count(prompt_summary_exec.as_str()))as f64 * crate::utils::AVG_TOKENS_PER_WORD;
    if estimated_tokens < llm_gen.num_context as f64
    {
        debug!("Generating summary for entire text content since estimated context length = {}",
            estimated_tokens);
        _ = raw_doc.generated_content.insert(
            "exec_summary".to_string(),
            generate_text_using_llm(
                llm_gen,"".to_string(), prompt_summary_exec.clone(), raw_doc.text.clone()
            )
        );
    } else {
        debug!("Generating part by part summary for this document since estimated context = {}",
            estimated_tokens);
        let mut summaries: String = String::new();
        let parts_summ: Vec<String> = raw_doc.text_parts.iter().map(|part| {
            if let Some(input_text) = part.get::<String>(&String::from("text")){
                let part_summ = generate_text_using_llm(
                    llm_gen,"".to_string(), prompt_summary_part.clone(), input_text.to_string()
                );
                summaries.push_str(&part_summ);
                part_summ
            }else{
                String::new()
            }
        }).collect();
        raw_doc.text_parts = raw_doc.text_parts.iter()
            .zip(parts_summ.iter())
            .map(|(part, input_text)| {
            let part_summ = format!("{}", input_text);
            let mut updated_part = part.clone();
            updated_part.insert("summary".to_string(), serde_json::Value::String(part_summ));
            updated_part
        }).collect();
        _ = raw_doc.generated_content.insert(
            "exec_summary".to_string(),
            generate_text_using_llm(
                llm_gen,"".to_string(), prompt_summary_exec.clone(), summaries
            )
        );
    }
}


#[cfg(test)]
mod tests {

    use config::Config;
    use log::{debug, error, info};
    use samvadsetu::llm::{LLMTextGenBuilder, LLMTextGenerator};
    use crate::document::Document;
    use crate::{get_plugin_cfg, get_cfg};
    use crate::cfg;
    use crate::plugins::{mod_summarize};
    use crate::plugins::mod_summarize::{generate_text_using_llm, update_doc};

    const TEST_CONTENT: &str = "Ahead of the 2024 US Presidential elections, Donald Trump declared that on January 20, when he takes oath of office, one of his first executive orders will be to impose 25 per cent tariff on Canada and Mexico on all its products coming into the US over their immigration policies. \n \n“As everyone is aware, thousands of people are pouring through Mexico and Canada, bringing Crime and Drugs at levels never seen before. Right now a Caravan coming from Mexico, composed of thousands of people, seems to be unstoppable in its quest to come through our currently Open Border. On January 20th, as one of my many first Executive Orders, I will sign all necessary documents to charge Mexico and Canada a 25% Tariff on ALL products coming into the United States, and its ridiculous Open Borders,” Donald Trump declared. \n‘Time for them to pay a very big price’: Donald Trump \n \nDonald Trump declared that the tariff will remain in effect until drugs and “illegal aliens” stop coming into the US. \n \n“This Tariff will remain in effect until such time as Drugs, in particular Fentanyl, and all Illegal Aliens stop this Invasion of our Country! Both Mexico and Canada have the absolute right and power to easily solve this long simmering problem. We hereby demand that they use this power, and until such time that they do, it is time for them to pay a very big price,” he added. \nDEPORTATION OF ILLEGAL ALIENS, BORDER SECURITY \n \nDonald Trump forecasted signing as many as 100 executive orders on his first day, covering deportations of, what he terms as, “illegal aliens”. To stop immigrants from coming into the US, Donald Trump is likely to order an executive order on immigration. \nClosing all borders is likely to be ordered. A border emergency is likely to be declared to impose harsh immigration restrictions, a report said. \n \n“We have to get the criminals out of our country,” he said once. \n \nAccording to reports, raids are being planned in Chicago and other parts of the country to hurry up deportations. \nROLLING BACK DIVERSITY, EQUITY, INCLUSION (DEI) INITIATIVES \n \nSince his inauguration coincides with the Martin Luther King Day, Donald Trump has also pledged to roll back diversity, equity and inclusion initiatives on his first day. Donald Trump plans to dismantle diversity, equity, and inclusion (DEI) programs, mandate employees to return to the office, and set the stage for staff reductions. \n \nConservatives have long criticised programs that give preference based on race, gender and sexual orientation, arguing they violate the Constitution. \n \nDonald Trump wants to unwind diversity, equity and inclusion programs known as DEI, require employees to come back to the office and lay the groundwork to reduce staff. \n \n“Expect shock and awe,” said Sen. Ted Cruz, R-Texas. \n \nMany of these orders will be designed to reverse or eliminate ones implemented by the Joe Biden administration. \n \nHe also plans to pardon some January 6 rioters on Day 1 \nACTION ON DRUG CARTELS, WITHDRAWAL FROM PARIS CLIMATE ACCORD \n \nA source familiar with Donald Trump's Day 1 agenda at White House said sone actions will be taken to classify drug cartels as “foreign terrorist organizations” and declare an emergency at the U.S.-Mexico border \n \nOther orders may aim to scrap Joe Biden's environmental regulations and withdraw the US from the Paris climate agreement, sources have said. \n \nMany of the executive orders are likely to face legal challenges.";

    #[test]
    fn test_llm_gen_direct_prompt() {
        /*
        let example_cfg = cfg::read_config_from_file("conf/newslookout.toml".to_string());
        let prompt = get_cfg!("prompt_summarize", example_cfg, "Summarize this text:\n");

        if let Some(llm_service) = get_plugin_cfg!("mod_summarize", "llm_service", example_cfg) {

            if let Some(mut llm_gen) = LLMTextGenBuilder::build_from_config(&example_cfg, llm_service.as_str()) {

                let gen_text = generate_text_using_llm(
                    &mut llm_gen, "".to_string(),
                    prompt,
                    TEST_CONTENT.to_string(),
                );
                println!("Generated summary:\n{:?}", gen_text);
            }
        }
        */
        assert!(true);
    }

    #[test]
    fn test_llm_gen_api_call(){
        /*
        let example_cfg = cfg::read_config_from_file("conf/newslookout.toml".to_string());

        if let Some(llm_service) = get_plugin_cfg!("mod_summarize", "llm_service", example_cfg) {

            if let Some(mut llm_gen) = LLMTextGenBuilder::build_from_config(&example_cfg, llm_service.as_str()) {
                llm_gen.user_prompt = get_cfg!("prompt_summarize", example_cfg, "Summarize this text:\n");
                println!("{:#?}", llm_gen);
                let llm_gen_result = llm_gen.generate_text("", TEST_CONTENT);
                if let Ok(llm_result) = llm_gen_result {
                    println!("{:?}", llm_result);
                } else if let Err(e) = llm_gen_result {
                    error!("When summarizing text: {}", e);
                    println!("ERROR When summarizing text: {}", e);
                }
            }
        }
        */
        assert!(false);
    }

    #[test]
    fn test_llm_gen_document() {
        /*
        let example_cfg = cfg::read_config_from_file("conf/newslookout.toml".to_string());
        if let Some(llm_service) = get_plugin_cfg!("mod_summarize", "llm_service", example_cfg) {
            println!("llm_service: {}", llm_service);
            if let Some(mut llm_gen) = LLMTextGenBuilder::build_from_config(&example_cfg, llm_service.as_str()) {
                let prompt_exec_summ = get_cfg!("prompt_summary_exec", example_cfg, "Summarize this text:\n");
                let prompt_part_summ = get_cfg!("prompt_summary_part", example_cfg, "Summarize this text:\n");
                let mut test_doc = Document::default();
                test_doc.text = TEST_CONTENT.to_string();
                update_doc(
                    &mut llm_gen,
                    &mut test_doc,
                    prompt_exec_summ,
                    prompt_part_summ
                );
                println!("Generated summary:\n{:?}", test_doc.generated_content.get("exec_summary"));
            }
        }
         */
        assert!(true);
    }

}
