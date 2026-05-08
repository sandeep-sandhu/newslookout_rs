// file: mod_en_yahoo_news.rs
// Plugin for Yahoo News website

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use chrono::Utc;
use config::Config;
use log::{error, info, warn};
use regex::Regex;
use scraper::{Html, Selector};

use crate::document::Document;
use crate::network::{make_http_client, read_network_parameters, http_get};
use crate::utils::{get_urls_from_database, clean_text};
use crate::cfg::get_database_filename;
use crate::content_extraction::extract_article_content;

pub const PLUGIN_NAME: &str = "mod_en_yahoo_news";
const PUBLISHER_NAME: &str = "Yahoo News";
const BASE_URL: &str = "https://news.yahoo.com/";
const MIN_CONTENT_LENGTH: usize = 400;

const STARTER_URLS: &[(&str, &str)] = &[
    ("https://news.yahoo.com/", "main"),
    ("https://finance.yahoo.com/news/", "finance"),
];

const SKIP_URL_PATTERNS: &[&str] = &[
    "/video/", "/photos/", "/sports/", "/entertainment/",
    "/lifestyle/", "/tech/", "/science/", "/oddities/",
    "#", "javascript:", "mailto:",
    "/originals/", "/podcasts/", "/newsletters/",
];

const VALID_URL_PATTERNS: &[&str] = &[
    "news.yahoo.com/",
    "finance.yahoo.com/news/",
];

pub fn run_worker_thread(tx: Sender<Document>, app_config: Arc<Config>) {
    info!("{}: Starting worker thread", PLUGIN_NAME);

    let network_params = read_network_parameters(&app_config);
    let client = make_http_client(&network_params);

    let database_filename = get_database_filename(&app_config);
    let already_retrieved = get_urls_from_database(&database_filename, PLUGIN_NAME);

    let min_quality: f32 = match app_config.get_float("content_extraction_min_quality") {
        Ok(q) => q as f32,
        Err(_) => 0.1,
    };

    for (starter_url, section_name) in STARTER_URLS {
        info!("{}: Fetching listing from {} (section: {})", PLUGIN_NAME, starter_url, section_name);

        let article_urls = get_article_urls_from_listing(
            starter_url,
            &client,
            &already_retrieved,
            network_params.retry_times,
            network_params.wait_time_min,
        );

        info!("{}: Found {} new article URLs in section {}", PLUGIN_NAME, article_urls.len(), section_name);

        for article_url in article_urls {
            match fetch_and_extract_article(
                &article_url,
                section_name,
                &client,
                min_quality,
                network_params.retry_times,
                network_params.wait_time_min,
            ) {
                Some(doc) if doc.text.len() >= MIN_CONTENT_LENGTH => {
                    if let Err(e) = tx.send(doc) {
                        error!("{}: Failed to send document: {}", PLUGIN_NAME, e);
                    }
                }
                Some(_) => warn!("{}: Skipping short article: {}", PLUGIN_NAME, article_url),
                None => warn!("{}: Could not extract content from: {}", PLUGIN_NAME, article_url),
            }
        }
    }

    info!("{}: Worker thread completed", PLUGIN_NAME);
}

fn get_article_urls_from_listing(
    listing_url: &str,
    client: &reqwest::blocking::Client,
    already_retrieved: &HashSet<String>,
    retry_times: usize,
    wait_time: usize,
) -> Vec<String> {
    let mut urls = Vec::new();

    let html = http_get(&listing_url.to_string(), client, retry_times, wait_time);
    if html.is_empty() {
        error!("{}: Failed to fetch listing page {}", PLUGIN_NAME, listing_url);
        return urls;
    }

    let document = Html::parse_document(&html);
    let link_selector = Selector::parse("a[href]").unwrap_or_else(|_| Selector::parse("a").unwrap());

    for element in document.select(&link_selector) {
        if let Some(href) = element.value().attr("href") {
            let full_url = if href.starts_with("http") {
                href.to_string()
            } else if href.starts_with('/') {
                format!("{}{}", BASE_URL.trim_end_matches('/'), href)
            } else {
                continue;
            };

            if !is_valid_article_url(&full_url) {
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

fn is_valid_article_url(url: &str) -> bool {
    // Yahoo News article URLs contain a long alphanumeric slug or numeric ID
    let has_valid = VALID_URL_PATTERNS.iter().any(|p| url.contains(p));
    if !has_valid {
        return false;
    }
    // Must have something after the domain (not just the homepage)
    let path_len = url.split('/').filter(|s| !s.is_empty()).count();
    if path_len < 3 {
        return false;
    }
    let has_skip = SKIP_URL_PATTERNS.iter().any(|p| url.contains(p));
    !has_skip
}

fn extract_unique_id_from_url(url: &str) -> String {
    // Yahoo article IDs often appear as long numeric suffix: e.g. article-slug-123456789.html
    if let Ok(re) = Regex::new(r"-(\d{8,})(?:\.html)?$") {
        if let Some(caps) = re.captures(url) {
            if let Some(m) = caps.get(1) {
                return m.as_str().to_string();
            }
        }
    }
    if let Some(last_seg) = url.trim_end_matches('/').rsplit('/').next() {
        return last_seg.trim_end_matches(".html").to_string();
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hasher);
    format!("{}", hasher.finish())
}

fn extract_article_body_with_css(html: &str) -> Option<String> {
    let document = Html::parse_document(html);

    let selectors = [
        "div.caas-body",
        "div[class*='article-body']",
        "div[class*='caas-content']",
        "article p",
        "div[class*='body'] p",
    ];

    for selector_str in &selectors {
        if let Ok(selector) = Selector::parse(selector_str) {
            let text: String = document
                .select(&selector)
                .map(|el| el.text().collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            if text.len() >= MIN_CONTENT_LENGTH {
                return Some(clean_text(text));
            }
        }
    }

    None
}

fn fetch_and_extract_article(
    url: &str,
    section_name: &str,
    client: &reqwest::blocking::Client,
    min_quality: f32,
    retry_times: usize,
    wait_time: usize,
) -> Option<Document> {
    let html = http_get(&url.to_string(), client, retry_times, wait_time);
    if html.is_empty() {
        return None;
    }

    let mut doc = Document::default();
    doc.module = PLUGIN_NAME.to_string();
    doc.plugin_name = PUBLISHER_NAME.to_string();
    doc.section_name = section_name.to_string();
    doc.url = url.to_string();
    doc.source_name = vec![PUBLISHER_NAME.to_string()];
    doc.unique_id = extract_unique_id_from_url(url);
    doc.publish_date = Utc::now().format("%Y-%m-%d").to_string();
    doc.publish_date_ms = Utc::now().timestamp();

    let html_doc = Html::parse_document(&html);
    if let Ok(sel) = Selector::parse("h1") {
        if let Some(el) = html_doc.select(&sel).next() {
            doc.title = clean_text(el.text().collect::<String>());
        }
    }
    if doc.title.is_empty() {
        if let Ok(sel) = Selector::parse("title") {
            if let Some(el) = html_doc.select(&sel).next() {
                doc.title = clean_text(el.text().collect::<String>());
            }
        }
    }

    let text = extract_article_content(&html, min_quality)
        .or_else(|| extract_article_body_with_css(&html))
        .unwrap_or_default();

    if text.len() < MIN_CONTENT_LENGTH {
        return None;
    }

    doc.text = text;
    Some(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_unique_id_numeric_suffix() {
        let url = "https://news.yahoo.com/us-economy-growth-123456789.html";
        let id = extract_unique_id_from_url(url);
        assert_eq!(id, "123456789");
    }

    #[test]
    fn test_extract_unique_id_slug_fallback() {
        let url = "https://news.yahoo.com/some-article/";
        let id = extract_unique_id_from_url(url);
        assert!(!id.is_empty());
    }

    #[test]
    fn test_is_valid_article_url_rejects_video() {
        assert!(!is_valid_article_url("https://news.yahoo.com/video/some-clip-12345.html"));
    }

    #[test]
    fn test_is_valid_article_url_rejects_homepage() {
        assert!(!is_valid_article_url("https://news.yahoo.com/"));
    }

    #[test]
    fn test_is_valid_article_url_accepts_article() {
        assert!(is_valid_article_url("https://news.yahoo.com/us-trade-deal-123456789.html"));
    }

    #[test]
    fn test_extract_body_with_css() {
        let html = r#"<html><body>
            <div class="caas-body">
                <p>First paragraph with enough text content to pass the minimum length check for this article test.</p>
                <p>Second paragraph providing additional body text for this test case with more words to increase length.</p>
                <p>Third paragraph ensures combined text exceeds four hundred characters minimum threshold requirement.</p>
                <p>Fourth paragraph with yet more content to definitively clear the minimum content length requirement set at four hundred characters.</p>
            </div>
        </body></html>"#;
        let result = extract_article_body_with_css(html);
        assert!(result.is_some());
        assert!(result.unwrap().contains("First paragraph"));
    }
}
