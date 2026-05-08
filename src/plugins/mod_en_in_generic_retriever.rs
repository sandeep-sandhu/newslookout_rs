// file: mod_en_in_generic_retriever.rs
// Enhanced generic retriever plugin using content-extractor-rl

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use chrono::Utc;
use config::Config;
use log::{debug, error, info, warn};
use regex::Regex;
use scraper::{Html, Selector};

use crate::document::Document;
use crate::network::{make_http_client, read_network_parameters, http_get};
use crate::utils::{get_urls_from_database, clean_text, check_and_fix_url};
use crate::cfg::{get_data_folder, get_database_filename};
use crate::content_extraction::extract_article_content;

pub const PLUGIN_NAME: &str = "mod_en_in_generic_retriever";
const PUBLISHER_NAME: &str = "Generic Retriever";
const MIN_CONTENT_LENGTH: usize = 400;

/// Configuration for a generic retriever site
pub struct GenericRetrieverConfig {
    pub plugin_name: &'static str,
    pub publisher_name: &'static str,
    pub base_url: &'static str,
    pub starter_urls: Vec<(&'static str, &'static str)>,
    pub skip_url_patterns: Vec<&'static str>,
    pub valid_url_patterns: Vec<&'static str>,
    pub css_selectors: Vec<&'static str>,
}

/// Generic run function that can be used to retrieve articles from any site
/// when provided with a configuration struct.
pub fn run_generic_retriever(
    tx: Sender<Document>,
    app_config: Arc<Config>,
    cfg: &GenericRetrieverConfig,
) {
    info!("{}: Starting generic worker thread", cfg.plugin_name);

    let network_params = read_network_parameters(&app_config);
    let client = make_http_client(&network_params);

    let database_filename = get_database_filename(&app_config);
    let already_retrieved = get_urls_from_database(&database_filename, cfg.plugin_name);

    let min_quality: f32 = match app_config.get_float("content_extraction_min_quality") {
        Ok(q) => q as f32,
        Err(_) => 0.1,
    };

    for (starter_url, section_name) in &cfg.starter_urls {
        info!("{}: Fetching listing from {} (section: {})", cfg.plugin_name, starter_url, section_name);

        let html = http_get(&starter_url.to_string(), &client, network_params.retry_times, network_params.wait_time_min);
        if html.is_empty() {
            error!("{}: Failed to fetch listing page {}", cfg.plugin_name, starter_url);
            continue;
        }

        let article_urls = extract_urls_from_html(
            &html,
            cfg.base_url,
            &already_retrieved,
            &cfg.skip_url_patterns,
            &cfg.valid_url_patterns,
        );

        info!("{}: Found {} new article URLs in section {}", cfg.plugin_name, article_urls.len(), section_name);

        for article_url in article_urls {
            let article_html = http_get(&article_url, &client, network_params.retry_times, network_params.wait_time_min);
            if article_html.is_empty() {
                warn!("{}: Could not fetch article: {}", cfg.plugin_name, article_url);
                continue;
            }

            let mut doc = Document::default();
            doc.module = cfg.plugin_name.to_string();
            doc.plugin_name = cfg.publisher_name.to_string();
            doc.section_name = section_name.to_string();
            doc.url = article_url.clone();
            doc.source_name = vec![cfg.publisher_name.to_string()];
            doc.unique_id = extract_uid_from_url(&article_url);
            doc.publish_date = Utc::now().format("%Y-%m-%d").to_string();
            doc.publish_date_ms = Utc::now().timestamp();

            // Extract title
            let html_doc = Html::parse_document(&article_html);
            if let Ok(title_sel) = Selector::parse("h1") {
                if let Some(title_el) = html_doc.select(&title_sel).next() {
                    doc.title = clean_text(title_el.text().collect::<String>());
                }
            }
            if doc.title.is_empty() {
                if let Ok(title_sel) = Selector::parse("title") {
                    if let Some(title_el) = html_doc.select(&title_sel).next() {
                        doc.title = clean_text(title_el.text().collect::<String>());
                    }
                }
            }

            // Extract content: try ML extractor first, then CSS selectors
            let text = extract_article_content(&article_html, min_quality)
                .or_else(|| extract_with_css(&article_html, &cfg.css_selectors, MIN_CONTENT_LENGTH))
                .unwrap_or_default();

            if text.len() < MIN_CONTENT_LENGTH {
                warn!("{}: Skipping article with too little content: {}", cfg.plugin_name, article_url);
                continue;
            }

            doc.text = text;

            if let Err(e) = tx.send(doc) {
                error!("{}: Failed to send document: {}", cfg.plugin_name, e);
            }
        }
    }

    info!("{}: Generic worker thread completed", cfg.plugin_name);
}

pub fn extract_urls_from_html(
    html: &str,
    base_url: &str,
    already_retrieved: &HashSet<String>,
    skip_patterns: &[&str],
    valid_patterns: &[&str],
) -> Vec<String> {
    let mut urls = Vec::new();

    let document = Html::parse_document(html);
    let link_selector = Selector::parse("a[href]").unwrap_or_else(|_| Selector::parse("a").unwrap());

    for element in document.select(&link_selector) {
        if let Some(href) = element.value().attr("href") {
            let full_url = if href.starts_with("http") {
                href.to_string()
            } else if href.starts_with('/') {
                format!("{}{}", base_url.trim_end_matches('/'), href)
            } else {
                continue;
            };

            let has_valid = valid_patterns.iter().any(|p| full_url.contains(p));
            if !has_valid {
                continue;
            }

            let has_skip = skip_patterns.iter().any(|p| full_url.contains(p));
            if has_skip {
                continue;
            }

            if already_retrieved.contains(&full_url) {
                continue;
            }

            urls.push(full_url);
        }
    }

    urls.sort();
    urls.dedup();
    urls
}

pub fn extract_with_css(html: &str, selectors: &[&str], min_length: usize) -> Option<String> {
    let document = Html::parse_document(html);

    for selector_str in selectors {
        if let Ok(selector) = Selector::parse(selector_str) {
            let text: String = document.select(&selector)
                .map(|el| el.text().collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");

            if text.len() >= min_length {
                return Some(clean_text(text));
            }
        }
    }

    None
}

pub fn extract_uid_from_url(url: &str) -> String {
    if let Ok(re) = Regex::new(r"[/\-](\d{5,})") {
        if let Some(caps) = re.captures(url) {
            if let Some(m) = caps.get(1) {
                return m.as_str().to_string();
            }
        }
    }
    if let Some(last_seg) = url.trim_end_matches('/').rsplit('/').next() {
        if !last_seg.is_empty() {
            return last_seg.to_string();
        }
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hasher);
    format!("{}", hasher.finish())
}

pub fn run_worker_thread(tx: Sender<Document>, app_config: Arc<Config>) {
    // Default generic retriever - does nothing without configuration
    info!("{}: Generic retriever needs specific site configuration to function.", PLUGIN_NAME);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_uid_from_url() {
        let url = "https://example.com/article/some-story-12345678/";
        let id = extract_uid_from_url(url);
        assert!(!id.is_empty());
        assert_eq!(id, "12345678");
    }

    #[test]
    fn test_extract_urls_from_html() {
        let html = r#"<html><body>
            <a href="/article/test-123456">Article 1</a>
            <a href="https://example.com/article/test-789012">Article 2</a>
            <a href="/contact">Contact</a>
        </body></html>"#;

        let already = std::collections::HashSet::new();
        let skip = vec!["/contact"];
        let valid = vec!["example.com/article/"];

        let urls = extract_urls_from_html(html, "https://example.com/", &already, &skip, &valid);
        assert!(urls.len() >= 1);
    }
}
