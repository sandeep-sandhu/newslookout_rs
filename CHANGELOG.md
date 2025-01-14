# Change Log

### Release 0.4.9
- Implemented mutexes to coordinate LLM service API usage
- Enhanced data structures used for data processing plugins
- Implemented Google's new Generative AI API service to support Gemini 2.0 Flash model
- Improved error handling of LLM API service requests and error logging

### Release 0.4.8
- Enabled overwriting of text parts by new split, if enabled in the config file
- Fixed document creation from PDF file (mod_offline)

### Release 0.4.7
- Fixed minor bugs

### Release 0.4.6
- Updated the crate documentation with additional details about the package

### Release 0.4.5
  - Added better logging to llm functions and modules using these (summarize)
  - Fixed compile time warnings throughout the project

### Release 0.4.4
  - Bug fixes and re-factoring.

### Release 0.4.3
  - Added new module for running arbitrary os commands with filename of retrieved document as the argument.

### Release 0.4.2
  - Fixed PLUGIN reference in llm module function - prepare_llm_parameters
  - In the same function, fixed the error message for retrieving value of overwrite key
  - Fixed the list of starter urls list for module rbi
  - Removed nested page listing urls in starter URLs, e.g. those for reports.
  - If PDF file exists and text attrib size is > 4 chars, then don't extract text from pdf
  - Summarize parts only if text + prompt size longer than max input tokens (e.g. 8100 tokens)
  - For the offline plugin, added folder name in config, to pick up details from a different folder than data folder.

### Release 0.4.1:
  - Bug fixes to 0.4.0

### Release 0.4.0:

  - Broke-up library (queue method start_pipeline) to individual components that define each thread process.
  - Changed newslook start message
  - Changed semver to 0.4.0
  - Change function name run_app to -> start_pipeline
  - Moved chatgpt, ollama, gemini codes out of plugin code into llm module
  - Moved logic to split text to llm module.
  - Added new starter URLs to module rbi

### Release 0.3.2
  - Bug fixes and patches for previous release
  - Add methods in chatgpt for api calls, use #tests to check these out.
  - Add methods in gemini for api calls, use #tests to check these out.
  - Before retrieving pdf, check if exists, dont retrieve and overwrite if so.
  - Move persist to disk to its own module, options: disk json, disk xml, database table, AWS bucket, etc. functionality to last data proc plugin.
  - Enhanced filename generation logic: limit file name length, after module and section name, keep only last 64 characters or url resource after stripping out special charcters, then append hash value of url, then append date at the end.
  - Removed docinfo, keep original complete document

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
