# Change Log


### Release 0.3.0
  - Added plugins to generate content using ChatGPT
  - Added plugins to generate content using Google Gemini
  - Updated Ollama plugin to support additional API calls
  - Segregated file save to disk to a separate data processing plugin of its own
  - Added cargo badge to README
  - Initialize thread specific random nos for a range and generate on each call to network fetch:
  - change ollama connect timeout to shorter time, 15 seconds.
  - In module, change word limit of text splitting to 600 words.
  - Add support for proxy
  - In RBI module, when saving html content, save only div element with class = Notification-content-wrap
  - init document with default "others" categories in classification field.
  - Clean-up recipient text at boundary - dear madam/sir, etc.
  - In RBI module, if last part is less than half of 600, then merge with second-last part.
  - Used common config based prompts for all llms to process documents
  - Split this into simpler parts and invoke prepare_prompt functions 
  - Refactored the LLM invocation method for processing the document to make it more generic.
  - Generate and set the unique filename at the time of downloading content
  - Enable saving partially processed document so that progress is not lost on interruptions or network failures


### Release 0.2.3
  - Fixed bug in run_app function
  - Fixed function and module visibility external to the crate

### Release 0.1.0
  - Initial Release
