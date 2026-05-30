// file: content_extraction.rs
// Article content extractor with three-level fallback:
//   1. RL model (DuelingDQN) via AgentFactory + ArticleExtractionEnvironment
//   2. BaselineExtractor (heuristic quality scoring)
//   3. CSS selector / HTML tag fallback

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, OnceLock};
use chrono::NaiveDate;
use log::{debug, error, info, warn};
use scraper::{ElementRef, Html, Selector};
use content_extractor_rl::{
    AgentFactory, ArticleExtractionEnvironment, BaselineExtractor,
    Config as RlConfig, RLAgent, SiteProfile, get_device,
};
use regex::Regex;
use crate::document::Document;
use crate::utils::{clean_text, get_text_from_element, to_local_datetime};

// ---------------------------------------------------------------------------
// Global extractor singleton (initialized once at startup)
// ---------------------------------------------------------------------------

static GLOBAL_EXTRACTOR: OnceLock<HtmlExtractor> = OnceLock::new();

/// Initialize the global HTML extractor with the optional RL model path.
/// Must be called once at application startup, after logging is configured.
/// Logs clearly which extraction level is active.
pub fn init_html_extractor(model_path: Option<&str>) {
    GLOBAL_EXTRACTOR.get_or_init(|| HtmlExtractor::new(model_path));
}

fn global_extractor() -> &'static HtmlExtractor {
    GLOBAL_EXTRACTOR.get_or_init(|| {
        // Lazy default if init_html_extractor was never called
        HtmlExtractor::new(None)
    })
}

// ---------------------------------------------------------------------------
// HtmlExtractor struct
// ---------------------------------------------------------------------------

/// Three-level HTML article extractor: RL model → BaselineExtractor → CSS selectors.
pub struct HtmlExtractor {
    /// Loaded DuelingDQN agent; None when model file is absent or fails to load.
    agent: Option<Arc<dyn RLAgent>>,
    config: RlConfig,
}

impl HtmlExtractor {
    /// Create a new extractor, optionally loading a DuelingDQN model from `model_path`.
    /// Logs the outcome clearly so operators know which extraction level is active.
    pub fn new(model_path: Option<&str>) -> Self {
        let config = RlConfig::default();

        let agent: Option<Arc<dyn RLAgent>> = if let Some(path) = model_path {
            let p = Path::new(path);
            if !p.exists() {
                warn!(
                    "RL extractor: model file '{}' not found. \
                     Using BaselineExtractor + CSS selector fallback.",
                    path
                );
                None
            } else {
                let device = get_device();
                match AgentFactory::load(
                    p,
                    config.state_dim,
                    config.num_discrete_actions,
                    config.num_continuous_params,
                    &device,
                ) {
                    Ok(boxed_agent) => {
                        info!(
                            "RL extractor: DuelingDQN model loaded from '{}'. \
                             Extraction order: RL model → BaselineExtractor → CSS selectors.",
                            path
                        );
                        Some(Arc::from(boxed_agent))
                    }
                    Err(e) => {
                        warn!(
                            "RL extractor: failed to load model '{}': {}. \
                             Using BaselineExtractor + CSS selector fallback.",
                            path, e
                        );
                        None
                    }
                }
            }
        } else {
            info!(
                "RL extractor: no model path configured. \
                 Using BaselineExtractor + CSS selector fallback."
            );
            None
        };

        Self { agent, config }
    }

    /// Extract article content using the three-level fallback chain.
    pub fn extract_content(&self, html: &str, url: &str, min_quality_score: f32) -> Option<String> {
        // Level 1: RL model
        if let Some(agent) = &self.agent {
            if let Some(text) = self.run_rl_extraction(agent.as_ref(), html, url) {
                debug!("Content extraction: RL model succeeded");
                return Some(text);
            }
            debug!("Content extraction: RL model produced no content, trying BaselineExtractor");
        }

        // Level 2: BaselineExtractor
        if let Some((text, quality)) = self.run_baseline_extraction(html) {
            if quality >= min_quality_score && !text.is_empty() {
                debug!("Content extraction: BaselineExtractor succeeded (quality={:.2})", quality);
                return Some(text);
            }
            debug!(
                "Content extraction: baseline quality {:.2} below threshold {:.2}",
                quality, min_quality_score
            );
        }

        // Level 3: CSS selector fallback
        match css_selector_extract(html) {
            Some((text, quality)) if quality >= min_quality_score && !text.is_empty() => {
                debug!("Content extraction: CSS selector fallback succeeded (quality={:.2})", quality);
                Some(text)
            }
            _ => {
                warn!(
                    "Content extraction: all three levels produced no content \
                     above quality threshold {:.2}",
                    min_quality_score
                );
                None
            }
        }
    }

    /// Run the RL model extraction loop, collecting the best text across all steps.
    fn run_rl_extraction(&self, agent: &dyn RLAgent, html: &str, url: &str) -> Option<String> {
        let baseline = BaselineExtractor::new(english_stopwords());
        let mut env = ArticleExtractionEnvironment::new(baseline, self.config.clone());

        let mut state = match env.reset(html, url.to_string(), None::<&SiteProfile>) {
            Ok(s) => s,
            Err(e) => {
                debug!("RL env reset error: {}", e);
                return None;
            }
        };

        let mut best_text = String::new();
        let mut best_quality = 0.0f32;
        let mut done = false;

        while !done {
            let (action, params) = match agent.select_action(&state, 0.0) {
                Ok(ap) => ap,
                Err(e) => {
                    debug!("RL select_action error: {}", e);
                    break;
                }
            };

            let (next_state, _reward, is_done, info) = match env.step((action, params)) {
                Ok(r) => r,
                Err(e) => {
                    debug!("RL env step error: {}", e);
                    break;
                }
            };

            if info.quality_score > best_quality && !info.text.is_empty() {
                best_quality = info.quality_score;
                best_text = info.text;
            }

            state = next_state;
            done = is_done;
        }

        if best_text.is_empty() { None } else { Some(best_text) }
    }

    /// Run BaselineExtractor and return (text, quality_score).
    fn run_baseline_extraction(&self, html: &str) -> Option<(String, f32)> {
        let extractor = BaselineExtractor::new(english_stopwords());
        match extractor.extract(html) {
            Ok(r) if !r.text.is_empty() => Some((r.text, r.quality_score)),
            Ok(_) => {
                debug!("BaselineExtractor returned empty text");
                None
            }
            Err(e) => {
                debug!("BaselineExtractor error: {}", e);
                None
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Stopwords used by BaselineExtractor's scoring heuristic
// ---------------------------------------------------------------------------

fn english_stopwords() -> HashSet<String> {
    [
        "the", "a", "an", "and", "or", "but", "in", "on", "at", "to", "for",
        "of", "with", "by", "from", "is", "are", "was", "were", "be", "been",
        "being", "have", "has", "had", "do", "does", "did", "will", "would",
        "could", "should", "may", "might", "shall", "can", "that", "this",
        "these", "those", "it", "its", "as", "if", "not", "no", "nor", "so",
        "yet", "both", "either", "neither", "each", "few", "more", "most",
        "other", "some", "such", "than", "then", "there", "when", "where",
        "who", "which", "how", "all", "any", "because", "before", "after",
        "while", "about", "into", "through", "during", "above", "below",
        "between", "among", "against", "he", "she", "they", "we", "you",
        "his", "her", "their", "our", "your", "my", "me", "him", "us", "them",
        "what", "said", "also", "just", "like", "up", "out", "over", "new",
        "only", "one", "two", "first", "last", "long", "great", "little",
        "own", "right", "old", "big", "high", "different", "small", "large",
        "next", "early", "young", "important", "public", "private", "real",
        "i", "s", "t", "re", "ve", "ll", "d", "m", "its",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

// ---------------------------------------------------------------------------
// Level 3: CSS selector / HTML tag fallback
// ---------------------------------------------------------------------------

/// Candidate content block with quality metrics.
struct ContentBlock {
    text: String,
    word_count: usize,
}

/// Extract article text using heuristic CSS selectors.
/// Returns (text, quality_score) or None when no usable block is found.
fn css_selector_extract(html: &str) -> Option<(String, f32)> {
    let document = Html::parse_document(html);

    let article_selectors = [
        "article",
        "[itemprop='articleBody']",
        "[class*='article-body']",
        "[class*='article_body']",
        "[class*='articleBody']",
        "[class*='story-body']",
        "[class*='story_body']",
        "[class*='entry-content']",
        "[class*='post-content']",
        "[class*='content-body']",
        "main",
        "[role='main']",
    ];

    let mut best_block: Option<ContentBlock> = None;

    for selector_str in &article_selectors {
        if let Ok(selector) = Selector::parse(selector_str) {
            for element in document.select(&selector) {
                let mut paragraphs = Vec::new();
                if let Ok(p_sel) = Selector::parse("p") {
                    for p in element.select(&p_sel) {
                        let text: String = p.text().collect();
                        let trimmed = text.trim().to_string();
                        if trimmed.len() > 30 {
                            paragraphs.push(trimmed);
                        }
                    }
                }

                let combined = if paragraphs.is_empty() {
                    element.text().collect::<String>().trim().to_string()
                } else {
                    paragraphs.join("\n\n")
                };

                let word_count = combined.split_whitespace().count();
                let should_replace = match &best_block {
                    None => word_count > 30,
                    Some(prev) => word_count > prev.word_count,
                };

                if should_replace && word_count > 30 {
                    best_block = Some(ContentBlock { text: combined, word_count });
                }
            }
        }
    }

    if let Some(block) = best_block {
        let quality = (block.word_count as f32 / 500.0).min(1.0);
        return Some((block.text, quality));
    }

    // Last resort: collect all <p> tags from the document
    if let Ok(p_sel) = Selector::parse("p") {
        let paragraphs: Vec<String> = document
            .select(&p_sel)
            .map(|p| p.text().collect::<String>().trim().to_string())
            .filter(|t| t.len() > 30)
            .collect();

        if !paragraphs.is_empty() {
            let text = paragraphs.join("\n\n");
            let word_count = text.split_whitespace().count();
            // Lower multiplier (0.5×) signals this is a rough last-resort fallback
            let quality = (word_count as f32 / 500.0).min(1.0) * 0.5;
            return Some((text, quality));
        }
    }

    None
}

fn extract_title_from_html(html: &str) -> Option<String> {
    let document = Html::parse_document(html);

    if let Ok(h1_sel) = Selector::parse("h1") {
        if let Some(h1) = document.select(&h1_sel).next() {
            let title: String = h1.text().collect::<String>().trim().to_string();
            if !title.is_empty() {
                return Some(title);
            }
        }
    }

    if let Ok(og_sel) = Selector::parse("meta[property='og:title']") {
        if let Some(meta) = document.select(&og_sel).next() {
            if let Some(content) = meta.value().attr("content") {
                let title = content.trim().to_string();
                if !title.is_empty() {
                    return Some(title);
                }
            }
        }
    }

    if let Ok(title_sel) = Selector::parse("title") {
        if let Some(title_el) = document.select(&title_sel).next() {
            let title: String = title_el.text().collect::<String>().trim().to_string();
            if !title.is_empty() {
                return Some(title);
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Extract article content from HTML using the three-level fallback chain.
///
/// Initializes the global extractor on first call if `init_html_extractor`
/// was not called at startup (no RL model, baseline + CSS fallback only).
///
/// # Arguments
/// * `html`              – Full HTML of the page.
/// * `min_quality_score` – Minimum quality score (0.0–1.0) to accept content.
pub fn extract_article_content(html: &str, min_quality_score: f32) -> Option<String> {
    global_extractor().extract_content(html, "", min_quality_score)
}

/// Extract article content from HTML, providing the source URL for RL domain features.
pub fn extract_article_content_with_url(html: &str, url: &str, min_quality_score: f32) -> Option<String> {
    global_extractor().extract_content(html, url, min_quality_score)
}

/// Extract the article title from HTML.
///
/// Tries the BaselineExtractor's metadata first; falls back to h1 → og:title → title.
pub fn extract_article_title(html: &str) -> Option<String> {
    let extractor = BaselineExtractor::new(english_stopwords());
    match extractor.extract(html) {
        Ok(result) => result.title.filter(|t| !t.is_empty()),
        Err(_) => None,
    }
    .or_else(|| extract_title_from_html(html))
}

/// Extract plain text from HTML using the three-level extractor chain.
///
/// Returns an empty string when no content can be extracted.
pub fn extract_text_from_html(html_content: &str) -> String {
    global_extractor()
        .extract_content(html_content, "", 0.0)
        .unwrap_or_else(|| {
            // Hard fallback: collect all text from root element
            let html_root = scraper::Html::parse_document(html_content);
            get_text_from_element(html_root.root_element())
        })
}

// ---------------------------------------------------------------------------
// HTML to Markdown converter
// ---------------------------------------------------------------------------

/// Convert an HTML string to Markdown text.
/// Handles headings, paragraphs, lists, links, bold/italic, tables, and blockquotes.
/// Skips script, style, nav, and head elements.
pub fn html_to_markdown(html: &str) -> String {
    let doc = Html::parse_document(html);
    let mut buf = String::new();
    convert_element_to_markdown(doc.root_element(), &mut buf, 0);
    // Collapse runs of 3+ newlines down to 2
    let re = Regex::new(r"\n{3,}").unwrap();
    re.replace_all(buf.trim(), "\n\n").to_string()
}

fn convert_element_to_markdown(element: ElementRef, buf: &mut String, list_depth: usize) {
    for child in element.children() {
        match child.value() {
            scraper::node::Node::Text(text) => {
                let t: &str = &text.text;
                let collapsed = t.split_whitespace().collect::<Vec<_>>().join(" ");
                if !collapsed.is_empty() {
                    buf.push_str(&collapsed);
                    buf.push(' ');
                }
            }
            scraper::node::Node::Element(el) => {
                // Clone the small strings we need before wrapping (avoids borrow conflict)
                let tag = el.name().to_string();
                let href = el.attr("href").map(|s| s.to_string());
                if let Some(child_elem) = ElementRef::wrap(child) {
                    match tag.as_str() {
                        "script" | "style" | "nav" | "head" | "footer" => {}
                        "h1" => {
                            buf.push_str("\n\n# ");
                            convert_element_to_markdown(child_elem, buf, list_depth);
                            buf.push('\n');
                        }
                        "h2" => {
                            buf.push_str("\n\n## ");
                            convert_element_to_markdown(child_elem, buf, list_depth);
                            buf.push('\n');
                        }
                        "h3" => {
                            buf.push_str("\n\n### ");
                            convert_element_to_markdown(child_elem, buf, list_depth);
                            buf.push('\n');
                        }
                        "h4" | "h5" | "h6" => {
                            buf.push_str("\n\n#### ");
                            convert_element_to_markdown(child_elem, buf, list_depth);
                            buf.push('\n');
                        }
                        "p" => {
                            buf.push_str("\n\n");
                            convert_element_to_markdown(child_elem, buf, list_depth);
                            buf.push_str("\n\n");
                        }
                        "br" => buf.push('\n'),
                        "ul" | "ol" => {
                            buf.push('\n');
                            convert_element_to_markdown(child_elem, buf, list_depth + 1);
                            buf.push('\n');
                        }
                        "li" => {
                            let indent = "  ".repeat(list_depth.saturating_sub(1));
                            buf.push_str(&format!("\n{}* ", indent));
                            convert_element_to_markdown(child_elem, buf, list_depth);
                        }
                        "strong" | "b" => {
                            buf.push_str("**");
                            convert_element_to_markdown(child_elem, buf, list_depth);
                            if buf.ends_with(' ') { buf.pop(); }
                            buf.push_str("** ");
                        }
                        "em" | "i" => {
                            buf.push('*');
                            convert_element_to_markdown(child_elem, buf, list_depth);
                            if buf.ends_with(' ') { buf.pop(); }
                            buf.push_str("* ");
                        }
                        "a" => {
                            let link_target = href.as_deref().unwrap_or("#");
                            buf.push('[');
                            convert_element_to_markdown(child_elem, buf, list_depth);
                            if buf.ends_with(' ') { buf.pop(); }
                            buf.push_str("](");
                            buf.push_str(link_target);
                            buf.push(')');
                        }
                        "table" => {
                            buf.push('\n');
                            convert_element_to_markdown(child_elem, buf, list_depth);
                            buf.push('\n');
                        }
                        "tr" => {
                            buf.push_str("\n| ");
                            convert_element_to_markdown(child_elem, buf, list_depth);
                            buf.push('|');
                        }
                        "td" | "th" => {
                            convert_element_to_markdown(child_elem, buf, list_depth);
                            buf.push_str(" | ");
                        }
                        "hr" => buf.push_str("\n---\n"),
                        "blockquote" => {
                            buf.push_str("\n> ");
                            convert_element_to_markdown(child_elem, buf, list_depth);
                            buf.push('\n');
                        }
                        _ => convert_element_to_markdown(child_elem, buf, list_depth),
                    }
                }
            }
            _ => {}
        }
    }
}

/// Extract document details from a row of news article listings produced by liferay portal.
pub fn extract_doc_from_row(row_each: ElementRef, source_url: &str) -> Document {

    let alink_selector = scraper::Selector::parse("a.mtm_list_item_heading").unwrap();
    let date_selector = scraper::Selector::parse("div.notification-date>span").unwrap();
    let doctitle_selector = scraper::Selector::parse("span.mtm_list_item_heading").unwrap();
    let pdf_link_selector = scraper::Selector::parse("a.matomo_download").unwrap();
    let description_snippet_selector = scraper::Selector::parse("div.notifications-description p").unwrap();

    let mut this_new_doc = Document::default();

    this_new_doc.classification = HashMap::from([
        ("channel".to_string(), "other".to_string()),
        ("customer_type".to_string(), "other".to_string()),
        ("function".to_string(), "other".to_string()),
        ("market_type".to_string(), "other".to_string()),
        ("occupation".to_string(), "other".to_string()),
        ("product_type".to_string(), "other".to_string()),
        ("risk_type".to_string(), "other".to_string()),
        ("doc_type".to_string(), "regulatory-notification".to_string()),
    ]);
    let mut date_str = String::from("");

    let snippet_regex: Regex = Regex::new(
        r"(RBI[/A-Z]+\d{4}-\d{2,4}/\d*)(.+\d{4}-\d{2,4}[ ]*)((January|February|March|April|May|June|July|August|September|October|November|December)[\d ]+,[\d ]+)(.+)(Madam|Madam[ ]*/[ ]*Dear Sir|Dear Sir/|Dear Sir /|Madam / Dear Sir|Madam / Sir|$)"
    ).unwrap();

    for alink_elem in row_each.select(&alink_selector) {
        if let Some(href) = alink_elem.value().attr("href") {
            this_new_doc.url = href.parse().unwrap();
        }
    }

    for date_div_elem in row_each.select(&date_selector) {
        date_str = clean_text(date_div_elem.inner_html());
        match NaiveDate::parse_from_str(date_str.as_str(), "%b %d, %Y") {
            Ok(naive_date) => {
                this_new_doc.publish_date_ms = to_local_datetime(naive_date).timestamp();
                this_new_doc.publish_date = naive_date.format("%Y-%m-%d").to_string();
            },
            Err(date_err) => {
                error!("Could not parse date '{}', error: {}", date_str.as_str(), date_err)
            }
        }
        debug!("From url {} , identified date = {}, timestamp = {}",
               this_new_doc.url, date_str, this_new_doc.publish_date_ms);
    }

    for title_span_elem in row_each.select(&doctitle_selector) {
        this_new_doc.title = clean_text(get_text_from_element(title_span_elem));
        this_new_doc.links_inward = vec![source_url.to_string()];
        debug!("Identified title: '{}' for url {}", this_new_doc.title, this_new_doc.url);
    }

    let mut snippet_text = String::from(" ");
    for snippet_elem in row_each.select(&description_snippet_selector) {
        let description_snippet = clean_text(get_text_from_element(snippet_elem))
            .replace("\r\n", " ")
            .replace("\n", " ");
        snippet_text.push_str(" ");
        snippet_text.push_str(description_snippet.as_str());
        debug!("Retrieving parts from inner elements: {}", snippet_text);
        if let Some(caps) = snippet_regex.captures(snippet_text.as_str()) {
            let id_prefix = caps.get(1).unwrap().as_str();
            this_new_doc.unique_id = clean_text(caps.get(2).unwrap().as_str().to_string());
            let pubdate_longformat_str = caps.get(3).unwrap().as_str();
            this_new_doc.recipients = caps.get(5).unwrap().as_str().to_string();
            debug!("\tid_prefix: {},\n unique_id: {},\n pubdate_longformat_str: {},\n recipients: {}",
                   id_prefix, this_new_doc.unique_id, pubdate_longformat_str, this_new_doc.recipients);
        }
    }

    for pdf_url_elem in row_each.select(&pdf_link_selector) {
        if let Some(href) = pdf_url_elem.value().attr("href") {
            this_new_doc.pdf_url = href.parse().unwrap();
        }
    }

    this_new_doc
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- HtmlExtractor struct tests ---

    #[test]
    fn test_html_extractor_new_no_model() {
        let extractor = HtmlExtractor::new(None);
        assert!(extractor.agent.is_none(), "No agent expected when model_path is None");
    }

    #[test]
    fn test_html_extractor_new_nonexistent_path() {
        let extractor = HtmlExtractor::new(Some("/nonexistent/path/model.safetensors"));
        assert!(extractor.agent.is_none(), "Agent should be None for missing model file");
    }

    #[test]
    fn test_html_extractor_extract_content_empty() {
        let extractor = HtmlExtractor::new(None);
        let result = extractor.extract_content("<html><body></body></html>", "", 0.0);
        assert!(result.is_none() || result.unwrap_or_default().is_empty());
    }

    #[test]
    fn test_html_extractor_extract_content_article_tag() {
        let extractor = HtmlExtractor::new(None);
        let html = r#"<html><body>
            <article>
                <h1>Test Article Title</h1>
                <p>This is the first paragraph with meaningful content for extraction.</p>
                <p>This is the second paragraph with more content for the article body.</p>
                <p>Third paragraph adds words to raise the quality score above zero.</p>
                <p>Fourth paragraph ensures enough text to pass the minimum threshold.</p>
                <p>Fifth paragraph with additional content for the test case result.</p>
            </article>
        </body></html>"#;
        let result = extractor.extract_content(html, "", 0.0);
        assert!(result.is_some(), "Should extract content from <article> tag");
        assert!(result.unwrap().contains("first paragraph"));
    }

    #[test]
    fn test_html_extractor_quality_threshold() {
        let extractor = HtmlExtractor::new(None);
        let html = r#"<html><body><article><p>Too short.</p></article></body></html>"#;
        let result = extractor.extract_content(html, "", 0.9);
        assert!(result.is_none(), "Short content should not pass high quality threshold");
    }

    #[test]
    fn test_html_extractor_css_fallback() {
        let extractor = HtmlExtractor::new(None);
        let html = r#"<html><body>
            <div class="page">
                <p>A first standalone paragraph with enough words to count as content here.</p>
                <p>A second standalone paragraph providing additional text for extraction.</p>
                <p>Third paragraph to ensure enough total content for extraction test.</p>
            </div>
        </body></html>"#;
        let result = extractor.extract_content(html, "", 0.0);
        assert!(result.is_some(), "Should fall back to CSS/paragraph extraction");
    }

    // --- Legacy public API tests ---

    #[test]
    fn test_extract_article_content_empty_html() {
        let result = extract_article_content("<html><body></body></html>", 0.0);
        assert!(result.is_none() || result.unwrap_or_default().is_empty());
    }

    #[test]
    fn test_extract_article_content_with_article_tag() {
        let html = r#"<html><body>
            <article>
                <h1>Test Article Title</h1>
                <p>This is the first paragraph of the article with some meaningful content here.</p>
                <p>This is the second paragraph with more content for the article body text.</p>
                <p>This is the third paragraph of additional content for the test case result.</p>
                <p>Fourth paragraph adds more words so the quality score rises above zero.</p>
                <p>Fifth paragraph ensures we have enough text to pass the minimum threshold.</p>
            </article>
        </body></html>"#;
        let result = extract_article_content(html, 0.0);
        assert!(result.is_some(), "Should extract content from <article> tag");
        let content = result.unwrap();
        assert!(content.contains("first paragraph"));
    }

    #[test]
    fn test_extract_article_content_fallback_paragraphs() {
        let html = r#"<html><body>
            <div class="page">
                <p>A first standalone paragraph with enough words to be counted as content.</p>
                <p>A second standalone paragraph providing additional text for the extractor.</p>
                <p>Third paragraph to ensure we have enough total content for extraction.</p>
            </div>
        </body></html>"#;
        let result = extract_article_content(html, 0.0);
        assert!(result.is_some(), "Should fall back to paragraph extraction");
    }

    #[test]
    fn test_extract_article_title_h1() {
        let html = r#"<html><head><title>Page Title</title></head><body>
            <h1>Article Heading</h1>
        </body></html>"#;
        let title = extract_article_title(html);
        assert!(title.is_some());
        let t = title.unwrap();
        assert!(!t.is_empty());
    }

    #[test]
    fn test_extract_article_title_og_meta() {
        let html = r#"<html><head>
            <meta property="og:title" content="Open Graph Title"/>
            <title>Site Name</title>
        </head><body><p>Content here.</p></body></html>"#;
        let title = extract_article_title(html);
        assert!(title.is_some());
    }

    #[test]
    fn test_extract_article_title_fallback_to_title_tag() {
        let html = r#"<html><head><title>Page Title - Site Name</title></head><body>
            <p>Some content here without an h1.</p>
        </body></html>"#;
        let title = extract_article_title(html);
        assert!(title.is_some());
        assert!(title.unwrap().contains("Page Title"));
    }

    #[test]
    fn test_english_stopwords_not_empty() {
        let sw = english_stopwords();
        assert!(sw.len() > 30);
        assert!(sw.contains("the"));
        assert!(sw.contains("and"));
    }

    #[test]
    fn test_quality_threshold_respected() {
        let html = r#"<html><body><article><p>Too short.</p></article></body></html>"#;
        let result = extract_article_content(html, 0.9);
        assert!(result.is_none(), "Short content should not pass high quality threshold");
    }

    #[test]
    fn test_extract_text_from_html_nonempty() {
        let html = r#"<html><body>
            <article>
                <p>This paragraph has enough words to survive extraction process.</p>
                <p>Second paragraph also contributes to the total word count here.</p>
            </article>
        </body></html>"#;
        let text = extract_text_from_html(html);
        assert!(!text.is_empty(), "extract_text_from_html should return non-empty text");
    }

    #[test]
    fn test_css_selector_extract_article() {
        let html = r#"<html><body>
            <article>
                <p>First paragraph with enough content to be extracted properly here.</p>
                <p>Second paragraph also has content worth extracting in this test case.</p>
                <p>Third paragraph ensures the word count is sufficient for this test.</p>
            </article>
        </body></html>"#;
        let result = css_selector_extract(html);
        assert!(result.is_some(), "css_selector_extract should find content in <article>");
        let (text, quality) = result.unwrap();
        assert!(!text.is_empty());
        assert!(quality > 0.0);
    }

    #[test]
    fn test_css_selector_extract_empty() {
        let result = css_selector_extract("<html><body></body></html>");
        assert!(result.is_none(), "Empty HTML should return None");
    }
}
