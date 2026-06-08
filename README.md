
# Newslookout

[![build](https://github.com/sandeep-sandhu/newslookout_rs/actions/workflows/rust.yml/badge.svg)](https://github.com/sandeep-sandhu/newslookout_rs/actions) ![Crates.io Downloads (latest version)](https://img.shields.io/crates/dv/newslookout) ![Crate version](https://img.shields.io/crates/v/newslookout.svg)

A light-weight, multithreaded web scraping platform for scanning and processing news articles. It is a Rust port of the Python [NewsLookout application](https://github.com/sandeep-sandhu/NewsLookout).

Runs in batch mode with minimal resources (single-core CPU, under 4 GB RAM) and saves extracted news articles as structured JSON files.

---

## Table of Contents

1. [Architecture](#architecture)
2. [JSON Output Format](#json-output-format)
3. [Content Extraction](#content-extraction)
4. [Retriever Plugins](#retriever-plugins)
5. [Data Processing Plugins](#data-processing-plugins)
6. [Quick Start](#quick-start)
7. [Configuration](#configuration)
8. [Adding Custom Plugins](#adding-custom-plugins)
9. [Building and Testing](#building-and-testing)
10. [Dependencies](#dependencies)

---

## Architecture

The pipeline is organised in three stages that run concurrently:

```
  Retriever 1 ──┐
  Retriever 2 ──┼──► [inter-thread queue] ──► DataProc 1 ──► DataProc 2 ──► ... ──► Output
  Retriever 3 ──┘
```

1. **Retriever plugins** run in parallel threads. Each plugin fetches article listings from its assigned website, visits each article URL, extracts the article text, and sends a populated `Document` struct into the shared queue.

2. **Data processing plugins** form a sequential chain. Each plugin receives a document from the previous stage, processes or enriches it (classification, deduplication, summarisation, etc.), and passes it on.

3. **Output** — the final `Document` is serialised to disk as a JSON file. Processed URLs are recorded in an SQLite database so they are not fetched again on subsequent runs.

**Thread model:**
- All retriever threads write to a single `mpsc::Sender<Document>`.
- The data-processing chain uses paired `mpsc` channels: the output of each plugin is the input of the next.
- The `BinaryHeap` priority queue ensures data-processing plugins run in priority order (lower number = higher priority).

---

## JSON Output Format

Each article is saved as a JSON file with the following structure:

```json
{
  "sourceName":  ["BBC News"],
  "pubdate":     "2024-03-15",
  "text":        "Full article body text...",
  "title":       "Article headline",
  "URL":         "https://www.bbc.com/news/business-...",
  "keywords":    ["economy", "gdp", "growth"],
  "industries":  [],
  "uniqueID":    "12345678",
  "module":      "mod_en_bbc"
}
```

Field descriptions:

| Field        | Type            | Description                                          |
|--------------|-----------------|------------------------------------------------------|
| `sourceName` | `[String]`      | Publisher name(s), e.g. `["BBC News"]`               |
| `pubdate`    | `String`        | Publication date in `YYYY-MM-DD` format             |
| `text`       | `String`        | Plain-text article body                             |
| `title`      | `String`        | Article headline                                    |
| `URL`        | `String`        | Canonical article URL                               |
| `keywords`   | `[String]`      | Keywords extracted from article metadata            |
| `industries` | `[String]`      | Industry classifications (populated by classifier) |
| `uniqueID`   | `String`        | Unique article identifier (numeric ID or URL hash) |
| `module`     | `String`        | Name of the retriever plugin that fetched this      |

The `Document` struct also retains many more internal fields (HTML content, text parts, classification map, etc.) used during the pipeline; only the fields above are written to the output JSON file.

---

## Content Extraction

Article body text is extracted using a two-stage fallback strategy implemented in `src/content_extraction.rs`:

1. **Heuristic CSS-selector extraction** — the `extract_article_content()` function tries a prioritised list of well-known article-body selectors (`article`, `[itemprop='articleBody']`, `[class*='article-body']`, `main`, etc.), collects paragraph text, and scores the result by word count. Results below the configured `content_extraction_min_quality` threshold are discarded.

2. **Site-specific CSS fallback** — each plugin defines its own `extract_article_body_with_css()` with selectors known to work for that particular website.

> **Note on `content-extractor-rl`:** The crate `content-extractor-rl = "0.1.1"` provides a reinforcement-learning-based article extractor (`BaselineExtractor` for heuristics, `AgentFactory` for DQN-based extraction). At the time of writing its pre-release dependency tree (`rand 0.10.0-rc`, `getrandom 0.4.0-rc`) does not compile cleanly with the stable Rust toolchain. The `src/content_extraction.rs` module exposes an **identical API** (`extract_article_content`, `extract_article_title`) so that once the crate's dependencies are stabilised it can be dropped in as a direct replacement with minimal code changes.

---

## Retriever Plugins

### Indian news sources

| Plugin name                  | Website                                  | Sections scraped        |
|------------------------------|------------------------------------------|-------------------------|
| `mod_en_in_rbi`              | Reserve Bank of India                    | Notifications           |
| `mod_en_in_business_std`     | Business Standard                        | Main page               |
| `mod_en_in_thehindu`         | The Hindu                                | Business, Economy       |
| `mod_en_in_livemint`         | Livemint                                 | Latest, Economy         |
| `mod_en_in_moneycontrol`     | Moneycontrol                             | News, Business          |
| `mod_en_in_timesofindia`     | Times of India                           | Business, India         |
| `mod_en_in_forbes`           | Forbes India                             | Main page               |
| `mod_en_in_indianexpress`    | Indian Express                           | Business, India         |
| `mod_en_in_indiankanoon`     | Indian Kanoon (legal)                    | Judgements              |

### International news sources

| Plugin name           | Website            | Sections scraped          |
|-----------------------|--------------------|---------------------------|
| `mod_en_bbc`          | BBC News           | Main, Business            |
| `mod_en_guardian`     | The Guardian       | World, Business           |
| `mod_en_ap_news`      | Associated Press   | Business, World news      |

### Offline / generic

| Plugin name                  | Description                                       |
|------------------------------|---------------------------------------------------|
| `mod_offline_docs`           | Reads existing PDF/JSON files from a local folder |
| `mod_en_in_generic_retriever`| Generic retriever — configure any URL in config   |

---

## Data Processing Plugins

Data processing plugins run **sequentially** after retrieval, in priority order (lower number = higher priority):

| Plugin name       | Priority | Description                                                          |
|-------------------|----------|----------------------------------------------------------------------|
| `split_text`      | 1        | Splits long articles into overlapping chunks for LLM processing     |
| `mod_dedupe`      | 4        | Detects near-duplicate articles using semantic embeddings           |
| `mod_classify`    | 5        | Classifies articles by industry/event type using FinBERT            |
| `mod_summarize`   | 7        | Generates executive summaries using LLM (Gemini / ChatGPT / Ollama)|
| `mod_vectorstore` | 11       | Writes text embeddings to a vector store                            |
| `mod_persist_data`| 13       | Serialises the document to a JSON file on disk                      |
| `mod_solrsubmit`  | 9        | Submits documents to Apache Solr                                    |
| `mod_cmdline`     | 99       | Passes the saved JSON file to a configured command-line tool        |

---

## Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
newslookout = "0.4.12"
```

Minimal usage:

```rust
use newslookout;

fn main() {
    let config_file = std::env::args().nth(1)
        .expect("Usage: my_app <config_file>");

    let app_config = newslookout::cfg::read_config_from_file(config_file);
    let docs = newslookout::load_and_run_pipeline(app_config);

    println!("Retrieved {} articles", docs.len());
}
```

Or run the bundled CLI binary:

```bash
cargo build --release
./target/release/newslookout_app conf/newslookout.toml
```

---

## Configuration

All behaviour is controlled by a single TOML config file. The repository ships `conf/newslookout.toml` as a documented example.

### Key settings

```toml
# Directories
data_dir   = "data/files"       # where JSON output files are written
models_dir = "models"           # directory for ML model weights
log_file   = "logs/newslookout.log"

# Network
fetch_timeout         = 60      # HTTP read timeout in seconds
connect_timeout       = 10
retry_count           = 3
retry_wait_fixed_sec  = 3
user_agent            = "Mozilla/5.0 ..."

# Crawl politeness / robots.txt
respect_robots_txt    = true    # honor each site's robots.txt before fetching article URLs;
                                # set to false to disable robots.txt checks across all sites
min_host_interval_sec = 3       # minimum seconds between consecutive fetches to the same host
                                # (shared per-host throttle across all retriever threads);
                                # falls back to wait_time_min if not set

# Content extraction
content_extraction_min_quality   = 0.1    # 0.0–1.0; lower = accept noisier extractions
content_extraction_model_file    = "models/dqn_model.safetensors"  # reserved for future RL model

# Logging
log_level = "INFO"   # DEBUG | INFO | WARN | ERROR
```

### Plugin activation

Each plugin entry in the `plugins` array has:

```toml
plugins = [
  { enabled=true,  name="mod_en_bbc",        type="retriever",      priority=2 },
  { enabled=true,  name="mod_en_in_thehindu",type="retriever",      priority=3 },
  { enabled=true,  name="split_text",        type="data_processor", priority=1,
    overwrite=false, min_word_limit_to_split=700, previous_part_overlap=70 },
  { enabled=true,  name="mod_persist_data",  type="data_processor", priority=13,
    destination="file", file_format="json" },
]
```

Set `enabled=false` to disable a plugin without removing it from the config.

### LLM API configuration

```toml
[llm_apis."gemini"]
model_name      = "gemini-1.5-flash"
api_url         = "https://generativelanguage.googleapis.com/v1beta/models"
max_context_len = 16384
max_gen_tokens  = 8192
temperature     = 0.0
```

Supported LLM backends: `gemini`, `google_genai`, `chatgpt`, `ollama`.

Set the corresponding API key as an environment variable before running:

```bash
export GEMINI_API_KEY="..."
export OPENAI_API_KEY="..."
```

---

## Adding Custom Plugins

### Retriever plugin

1. Create `src/plugins/mod_my_site.rs` using an existing plugin as a template (e.g. `mod_en_bbc.rs`).
2. Implement `pub fn run_worker_thread(tx: Sender<Document>, app_config: Arc<Config>)`.
3. Declare it in `src/lib.rs`:
   ```rust
   pub mod plugins {
       pub mod mod_my_site;
       // ...
   }
   ```
4. Register it in `src/pipeline.rs` inside `load_retriever_plugins`:
   ```rust
   "mod_my_site" => {
       retriever_plugins.push(RetrieverPlugin {
           name: plugin_name,
           priority,
           enabled: plugin_enabled,
           method: mod_my_site::run_worker_thread,
       });
       continue;
   },
   ```
5. Add it to `conf/newslookout.toml`:
   ```toml
   { enabled=true, name="mod_my_site", type="retriever", priority=3 }
   ```

### Data processing plugin

1. Create `src/plugins/mod_my_processor.rs`.
2. Implement:
   ```rust
   pub fn process_data(
       tx: Sender<Document>,
       rx: Receiver<Document>,
       config: &Config,
       api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>,
   )
   ```
3. Register it in `src/lib.rs` and `src/pipeline.rs` following the same pattern.

### Helper macros (in `src/cfg.rs`)

```rust
// Read a string config value with a default
let value = get_cfg!("my_param", app_config, "default_value");

// Read an integer config value
let count = get_cfg_int!("max_pages", app_config, 10isize);

// Read a bool config value
let flag = get_cfg_bool!("save_html", app_config, false);

// Read a plugin-specific config parameter
let folder = get_plugin_cfg!("mod_offline_docs", "folder_name", app_config);
```

---

## Building and Testing

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Run all unit tests
cargo test

# Check for lint warnings (warnings are acceptable, errors must be fixed)
cargo clippy

# Run the application
./target/release/newslookout_app conf/newslookout.toml
```

### Pre-existing test failures

Two tests are intentionally failing as placeholders (they exist in unchanged pre-existing code):

- `plugins::mod_summarize::tests::test_llm_gen_api_call` — contains `assert!(false)` as a stub
- `plugins::split_text::tests::test_check_and_split_text` — expected value references old `split_by_word_count` behaviour

These do not affect any production code paths.

---

## Dependencies

| Crate             | Purpose                                             |
|-------------------|-----------------------------------------------------|
| `reqwest`         | Blocking HTTP/HTTPS client with cookie and gzip support |
| `scraper`         | CSS-selector HTML parsing                           |
| `serde` / `serde_json` | Serialisation / deserialisation of JSON       |
| `rusqlite`        | SQLite database for URL history tracking            |
| `chrono`          | Date/time handling                                  |
| `config`          | TOML configuration file parsing                     |
| `log` / `log4rs`  | Structured logging with file rotation               |
| `regex`           | Regular-expression URL pattern matching             |
| `lopdf` / `pdf-extract` | PDF content extraction                      |
| `clap`            | Command-line argument parsing                       |
| `samvadsetu`      | LLM API client (Gemini, ChatGPT, Ollama)            |
| `rand`            | Random wait times between HTTP requests             |

---

## Project Layout

```
newslookout_rs/
├── conf/
│   └── newslookout.toml        # Main configuration file
├── src/
│   ├── bin.rs                  # CLI entry point
│   ├── lib.rs                  # Library crate root
│   ├── pipeline.rs             # Thread orchestration, plugin loading
│   ├── document.rs             # Document struct + to_output_json()
│   ├── content_extraction.rs   # Article content extraction (heuristic)
│   ├── html_extract.rs         # HTML helper utilities
│   ├── network.rs              # HTTP client helpers
│   ├── utils.rs                # File, text, database utilities
│   ├── cfg.rs                  # Config access macros
│   ├── llm.rs                  # LLM API integration
│   └── plugins/
│       ├── mod_en_in_rbi.rs
│       ├── mod_en_in_business_standard.rs
│       ├── mod_en_in_thehindu.rs
│       ├── mod_en_in_livemint.rs
│       ├── mod_en_in_moneycontrol.rs
│       ├── mod_en_in_timesofindia.rs
│       ├── mod_en_in_forbes.rs
│       ├── mod_en_in_indianexpress.rs
│       ├── mod_en_in_indiankanoon.rs
│       ├── mod_en_bbc.rs
│       ├── mod_en_guardian.rs
│       ├── mod_en_ap_news.rs
│       ├── mod_en_in_generic_retriever.rs
│       ├── mod_offline_docs.rs
│       ├── split_text.rs
│       ├── mod_classify.rs
│       ├── mod_dedupe.rs
│       ├── mod_summarize.rs
│       ├── mod_vectorstore.rs
│       ├── mod_persist_data.rs
│       ├── mod_solrsubmit.rs
│       └── mod_cmdline.rs
├── data/
│   ├── files/                  # Output JSON files (created at runtime)
│   └── newslookout_urls.db     # SQLite URL history (created at runtime)
├── logs/                       # Log files (created at runtime)
├── models/                     # ML model weights (optional)
├── Cargo.toml
├── CHANGELOG.md
└── README.md
```

---

## Notice

This software is intended for demonstration and educational purposes only. Before using it to scrape any website, always consult that website's terms of use. The author is not liable for inappropriate use of this software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND.
