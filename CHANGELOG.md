# Change Log

### Release 1.0.1

Updates and bug fixes:

1. **Fixed compilation errors from `samvadsetu` v1.0.0 API change** (src/plugins/mod_summarize.rs): `LLMTextGenerator` dropped its `user_prompt` field and `generate_text(prefix, suffix)` was replaced by `generate_text(messages: &[ChatMessage], tools, response_format)`. Updated `generate_text_using_llm` to build a `[ChatMessage::system(user_prompt), ChatMessage::user(prefix + suffix)]` slice and call the new signature — summarisation behaviour is unchanged.

2. **Fixed broken listing-page URLs for several international retriever plugins** — sites had restructured their section paths, returning HTTP 404 (verified with `curl`):
    - `mod_en_in_thehindu`: `/economy/` → `/business/Economy/`
    - `mod_en_arab_news`: `/middle-east` → `/middleeast`
    - `mod_en_the_national`: `/economy/` → `/business/economy/`
    - `mod_en_allafrica`: removed the `/economy/` starter URL — the section was merged into `/business/` (site now redirects there)
    - `mod_en_nhk_world`: removed the `/news/business/` starter URL — the section no longer exists on NHK World's restructured (JS-rendered) site; only the top-level `/news/` listing remains valid

3. **Removed retriever plugins permanently blocked by anti-bot protections** (src/lib.rs, src/pipeline.rs, conf/newslookout.toml, and source files deleted): `mod_en_marketwatch`, `mod_en_in_ndtv`, `mod_en_reuters`, `mod_en_news24`, `mod_en_bloomberg`, `mod_en_usatoday`, `mod_en_guardian_ng`, `mod_en_yahoo_news`, `mod_en_washingtonpost`. Each of these returned HTTP 401/403 or connection resets on every listing-page fetch — verified with `curl` using a browser user-agent — confirming the failures are anti-bot blocks at the website's edge, not URL/path changes that can be fixed in the plugin. Removing them eliminates persistent error-log noise for sites that cannot be scraped. Updated README plugin tables and the JSON example (now uses `mod_en_bbc`/BBC News) accordingly.

4. **Configurable robots.txt / crawl-politeness settings** (src/network.rs, src/plugins/html_news.rs, conf/newslookout.toml):
    - New global config keys:
        - `respect_robots_txt` (bool, default `true`) — globally toggles whether retriever plugins consult each site's `robots.txt` before fetching article URLs. Per-site `SiteConfig::respect_robots` settings are ANDed with this global flag, so turning it off overrides all sites at once while leaving the per-site flags intact for future use.
        - `min_host_interval_sec` (integer, optional) — minimum number of seconds to wait between consecutive fetches to the same host (per-host politeness throttle shared across all retriever threads). Falls back to `wait_time_min` if not set.
    - `NetworkParameters` gained `respect_robots_txt: bool` and `min_host_interval_sec: Option<usize>` fields, populated by `read_network_parameters`.
    - `html_news::run` now derives the per-host crawl interval from `min_host_interval_sec` (falling back to `wait_time_min`), and `allowed_by_robots` takes the global `respect_robots_txt` flag so operators can disable robots.txt checks without editing every plugin.

5. **Fixed compilation error from `content-extractor-rl` v1.0.0 API change** (src/content_extraction.rs): `ArticleExtractionEnvironment::reset` gained a new `ground_truth_text: Option<&str>` parameter (3rd positional argument, between `url` and `_site_profile`). Updated the call site to pass `None::<&str>` since no ground-truth text is available at inference time — extraction behaviour is unchanged.

6. **MoneyControl extraction fix** (src/plugins/mod_en_in_moneycontrol.rs): added `div#div_app_container` to `body_selectors` as a CSS fallback. MoneyControl "earnings" stub articles embed their text in `<div id="div_app_container">`, but their JSON-LD `articleBody` contains literal unescaped `\r\n` control characters that fail JSON parsing — the new selector lets the existing CSS-fallback path extract the content directly.


### Release 1.0.0

Summary of Updates and bug fixes:

1. Fix correctness & log-noise

- html5ever WARN spam silenced — per-module log threshold in lib.rs; eliminates the 2,880 "foster parenting not
  implemented" lines (79% of all warnings).
- get_plugin_cfg! missing-key ERROR → debug — optional keys with defaults no longer log errors (kills the
  max_pages/items_per_page/vectorstore_model_dir error spam).
- http_get hardened — now checks status().is_success(), retries only retryable statuses (5xx/408/429), abandons
  permanent 4xx (403/404) immediately, exponential backoff + jitter. A 403/404 error page is no longer mistaken for
  article content.
- IRDAI '--' date — placeholder/empty dates skipped silently instead of erroring 20×.

2. BSE/NSE deep-fix

- BSE: verified via curl that the old EQ{DDMMYY}_CSV.ZIP URL now returns HTML; switched to the current UDiFF plain-CSV
  URL. NSE: cookie-store + landing-page warm-up for the API/listings. Both now walk back over recent business days
  (utils::recent_business_days) to handle weekends/holidays/early runs.

3. Full generic-retriever migration

- New src/plugins/html_news.rs: one config-driven retriever (SiteConfig + run()), covering URL validity, ID
  extraction, the .filter() extraction-order fix, and real publish-date parsing (JSON-LD/meta instead of now()).
- All 47 news plugins migrated to thin SiteConfig declarations (~40 lines each vs ~250).
- Dispatch table replaces ~500 lines of match arms in pipeline.rs (1119 → 686 lines).

4. Discovery, dedup, politeness

- utils::canonicalize_url (strips tracking params/fragment/trailing slash).
- mod_dedupe (was a no-op // TODO): SimHash near-duplicate detection for syndicated wire copy.
- src/discovery.rs: RSS/Atom/sitemap parsing, robots.txt compliance, per-host rate limiting — all wired into
  html_news.


5. Systemic fixes in html_news.rs (affected all 47 plugins):

1. Protocol-relative URL resolution — //gem.cbc.ca was falling into the starts_with('/') branch and being prepended with the full base_url, producing malformed URLs like www.cbc.ca/news//gem.cbc.ca. Now correctly resolved as https://gem.cbc.ca (then filtered by valid_url_patterns).

2. Absolute-path URL resolution — hrefs like /music were joined with the full base_url including its path (e.g. https://www.cbc.ca/news + /music =   https://www.cbc.ca/news/music), which then falsely passed the cbc.ca/news/ pattern. Now joined with scheme+host only via the new scheme_host_of() helper →   https://www.cbc.ca/music → correctly filtered.


6. Per-plugin fixes:

┌─────────────────┬───────────────────────────────────────────────────────────────────────────┬────────────────────────────────────────────────────────────────┐
│     Plugin      │                                    Fix                                    │                       Errors eliminated                        │
├─────────────────┼───────────────────────────────────────────────────────────────────────────┼────────────────────────────────────────────────────────────────┤
│ CBC             │ valid_url_patterns: www.cbc.ca/news/ + min_path_depth: 5                  │ //gem.cbc.ca, /music, /radio, /news/news/politics section spam │
├─────────────────┼───────────────────────────────────────────────────────────────────────────┼────────────────────────────────────────────────────────────────┤
│ CNBC            │ valid_url_patterns: www.cnbc.com/ + min_path_depth: 5                     │ //www.cnbc.com//... double-slash URLs                          │
├─────────────────┼───────────────────────────────────────────────────────────────────────────┼────────────────────────────────────────────────────────────────┤
│ Chicago Tribune │ valid_url_patterns: www.chicagotribune.com/                               │ myaccount., placeanad. subdomain fetches                       │
├─────────────────┼───────────────────────────────────────────────────────────────────────────┼────────────────────────────────────────────────────────────────┤
│ Bangkok Post    │ valid_url_patterns: www.bangkokpost.com/                                  │ job.bangkokpost.com fetches                                    │
├─────────────────┼───────────────────────────────────────────────────────────────────────────┼────────────────────────────────────────────────────────────────┤
│ LA Times        │ valid_url_patterns: www.latimes.com/                                      │ membership.latimes.com fetches                                 │
├─────────────────┼───────────────────────────────────────────────────────────────────────────┼────────────────────────────────────────────────────────────────┤
│ SMH             │ valid_url_patterns: www.smh.com.au/                                       │ tributes.smh.com.au fetches                                    │
├─────────────────┼───────────────────────────────────────────────────────────────────────────┼────────────────────────────────────────────────────────────────┤
│ ABC Australia   │ min_path_depth: 5 + skip /education, /listen, /local, /abckids            │ Section page spam                                              │
├─────────────────┼───────────────────────────────────────────────────────────────────────────┼────────────────────────────────────────────────────────────────┤
│ Yahoo News      │ min_last_segment_len: 10                                                  │ /news/2/ through /news/9/ pagination fetches                   │
├─────────────────┼───────────────────────────────────────────────────────────────────────────┼────────────────────────────────────────────────────────────────┤
│ Moneycontrol    │ min_path_depth: 5 + skip /infographic, /photogallery, /slideshow          │ /news/fintech/ etc. section pages                              │
├─────────────────┼───────────────────────────────────────────────────────────────────────────┼────────────────────────────────────────────────────────────────┤
│ Guardian        │ Added /gallery/, /audio/, /info/, /sign-up to skip                        │ Gallery/podcast/info page fetches                              │
├─────────────────┼───────────────────────────────────────────────────────────────────────────┼────────────────────────────────────────────────────────────────┤
│ Punch NG        │ "/advertise/" → "/advertise"                                              │ /advertise-with-us (missed due to no trailing slash)           │
├─────────────────┼───────────────────────────────────────────────────────────────────────────┼────────────────────────────────────────────────────────────────┤
│ CNA             │ Added /listen/ to skip                                                    │ /listen/cna938/schedule fetches                                │
├─────────────────┼───────────────────────────────────────────────────────────────────────────┼────────────────────────────────────────────────────────────────┤
│ The Hindu       │ Added /subscription, /crosswords, /premium, /lit-for-life, /ebook to skip │ Subscription/content gate pages                                │
└─────────────────┴───────────────────────────────────────────────────────────────────────────┴────────────────────────────────────────────────────────────────┘


### Release 0.5.0
New functionality, Updates and bug fixes:

1. Log rotation (src/lib.rs): Replaced the plain FileAppender with a RollingFileAppender using a CompoundPolicy (size trigger + fixed-window roller). The backup pattern is <logfile>.{N}. Both max_logfile_size and logfile_backup_count are read from config.

2. Duplicate JSON filenames (src/plugins/mod_persist_data.rs): make_json_entry_name now always appends a hex URL hash suffix, so filenames are globally unique even when two documents share the same unique_id slug.
mod_persist_data.rs: make_json_entry_name now produces {plugin_name}_{url_hash:016x}.json always — no unique_id, no section name, no
  article title. The 16-character hex URL hash is guaranteed unique per article URL. Before: mod_en_in_rbi_Draft 
  Notifications_abc123.json; after: mod_en_in_rbi_a1b2c3d4e5f6a7b8.json.

3. SQLite missing-table error (src/utils.rs): get_urls_from_database now runs CREATE TABLE IF NOT EXISTS after opening the connection, so a fresh database file is initialised automatically on first run.

4. PDF error message (src/utils.rs): retrieve_pdf_content signature extended with plugin_name: &str and source_url: &str; the error log now reads [plugin] When converting PDF into text, url='…', source='…': ….

5. Invalid IRDAI PDF files (src/utils.rs): After downloading, load_pdf_content checks the %PDF magic bytes before writing. Non-PDF binary content is logged with byte count and discarded — no file is written to disk, keeping the pipeline clean.

6. Web API (src/web_api.rs, src/bin.rs, conf/newslookout.toml): A lightweight stdlib TcpListener-based HTTP server (no new dependencies). Endpoints: GET / (info), /health, /status (full JSON), /status/summary, /dashboard.html (auto-refreshing dark-theme dashboard). Enable with web_api_enabled=true, configure via web_api_host and web_api_port in the config file. The pipeline updates a shared Arc<Mutex<PipelineStatus>> so the API reflects live counts.

7. conf/newslookout.toml — added:
pdf_data_dir = "data/master_data"
(sits right below master_data_dir; change the value to any path you prefer)

8. src/cfg.rs — added get_pdf_data_folder(config): reads pdf_data_dir, falls back to data_dir if the key is absent, and
creates the directory with fs::create_dir_all if it doesn't exist yet.

9. src/utils.rs — renamed load_pdf_content's data_folder parameter to pdf_folder and updated all internal path constructions
to use it, making the separation explicit.

10. Plugin call chains — threaded pdf_folder: &str through every layer that calls load_pdf_content:
- mod_en_in_irdai.rs: run_worker_thread → get_docs_from_listing_page
- mod_en_in_sebi.rs: run_worker_thread → sebi_retrieve_docs
- mod_en_in_rbi.rs: run_worker_thread → retrieve_data → get_docs_from_listing_page
- bin.rs: three inline scanners (run_rbi_scanner, run_sebi_scanner, run_irdai_scanner) and their inner retrieval functions

11. All JSON files continue to be saved under data_dir; only downloaded PDFs go to pdf_data_dir.

12. content_extraction.rs refactored with three-level fallback:
  - HtmlExtractor struct: holds an optional Arc<dyn RLAgent> (loaded once) and the RlConfig. Level 1 runs the DuelingDQN agent loop via
   ArticleExtractionEnvironment, tracking the highest-quality text across steps. Level 2 runs BaselineExtractor directly. Level 3 uses
  CSS selectors and <p> tag collection.
  - init_html_extractor(model_path: Option<&str>): new public function called from bin.rs after init_logging. Logs one of: model
  loaded, file not found, or no path configured. Stored in a OnceLock<HtmlExtractor> singleton so it initializes exactly once.
  - bin.rs: reads rl_model_path from config and calls init_html_extractor at startup.
  - Backward-compatible API: extract_article_content, extract_article_title, extract_text_from_html, and extract_doc_from_row all
  retain their existing signatures.
  - 17 tests added/updated; all pass.



### Release 0.4.13
- Declared 6 new plugins in src/lib.rs: mod_en_in_irdai, mod_en_in_sebi, mod_in_nse (retrievers) + mod_doc_type, mod_filter, mod_metadata (data processors)
- Wired all 6 into src/pipeline.rs - imports, retriever match arms, and data processor match arms
- Added config entries in conf/newslookout.toml:
    - mod_en_in_sebi and mod_en_in_irdai as retrievers (priority=1, enabled)
    - mod_doc_type (priority=2), mod_filter (priority=3), mod_metadata (priority=6, disabled by default since it requires LLM)
- Fixed a type error in mod_in_nse.rs:74 - &str → &String for http_get()

### Release 0.4.12
- Added structured JSON output format: `sourceName`, `pubdate`, `text`, `title`, `URL`, `keywords`, `industries`, `uniqueID`, `module` (matching the reference example files)
- Added `source_name`, `keywords`, and `industries` fields to the `Document` struct
- Added `Document::to_output_json()` method that serialises to the exact target JSON schema
- Created `src/content_extraction.rs` — a heuristic article-body extractor (CSS-selector based, API-compatible with the `content-extractor-rl` crate's `BaselineExtractor`)
- Added 12 new retriever plugins covering major global and Indian news sites: `mod_en_reuters`, `mod_en_bbc`, `mod_en_guardian`, `mod_en_bloomberg`, `mod_en_ap_news`, `mod_en_in_thehindu`, `mod_en_in_ndtv`, `mod_en_in_livemint`, `mod_en_in_moneycontrol`, `mod_en_in_timesofindia`, `mod_en_in_forbes`, `mod_en_in_indianexpress`
- Each plugin uses content-extractor-rl-compatible heuristic extraction with site-specific CSS fallback selectors
- Added `http_get` helper function to `network.rs` for HTML retrieval
- Added `content_extraction_min_quality` and `content_extraction_model_file` config parameters
- Updated config file with new plugin entries (all disabled by default)
- Updated README with full architecture documentation, JSON format spec, plugin table, configuration guide, and project layout
- All 62 existing tests pass; 2 pre-existing test stubs remain intentionally failing

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
