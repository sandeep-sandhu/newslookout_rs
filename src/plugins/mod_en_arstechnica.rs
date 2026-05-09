// file: mod_en_arstechnica.rs
// Plugin for Ars Technica (arstechnica.com)

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

pub const PLUGIN_NAME: &str = "mod_en_arstechnica";
const PUBLISHER_NAME: &str = "Ars Technica";
const BASE_URL: &str = "https://arstechnica.com";
const MIN_CONTENT_LENGTH: usize = 400;

const STARTER_URLS: &[(&str, &str)] = &[
    ("https://arstechnica.com/", "tech"),
    ("https://arstechnica.com/tech-policy/", "policy"),
    ("https://arstechnica.com/science/", "science"),
    ("https://arstechnica.com/cars/", "cars"),
];

const SKIP_URL_PATTERNS: &[&str] = &[
    "/gallery/", "#", "javascript:", "mailto:", "/author/", "/tag/", "/forums/",
];

const VALID_URL_PATTERNS: &[&str] = &[
    "arstechnica.com/",
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
                None => warn!("{}: Could not extract content: {}", PLUGIN_NAME, article_url),
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
    let has_skip = SKIP_URL_PATTERNS.iter().any(|p| url.contains(p));
    if has_skip { return false; }
    // Ars Technica article URLs have date /YYYY/MM/
    url.contains("arstechnica.com/") && url.contains("/20") && url.len() > 45
}

fn extract_unique_id_from_url(url: &str) -> String {
    // Ars Technica article URLs end with slug as last path segment
    if let Some(seg) = url.trim_end_matches('/').rsplit('/').next() {
        if !seg.is_empty() {
            return seg.to_string();
        }
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hasher);
    format!("{}", hasher.finish())
}

fn extract_article_body_with_css(html: &str) -> Option<String> {
    let document = Html::parse_document(html);

    let selectors = [
        "div.article-guts p",
        "section[class*='article-body'] p",
        "div[class*='post-content'] p",
        "article p",
        "main p",
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

    // Try to extract publication date from URL path: /YYYY/MM/
    if let Ok(re) = Regex::new(r"/(\d{4})/(\d{2})/") {
        if let Some(caps) = re.captures(url) {
            let y = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let mo = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            if !y.is_empty() {
                doc.publish_date = format!("{}-{:0>2}-01", y, mo);
            }
        }
    }

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
    fn test_extract_unique_id() {
        let url = "https://arstechnica.com/tech-policy/2024/03/some-article-slug/";
        let id = extract_unique_id_from_url(url);
        assert_eq!(id, "some-article-slug");
    }

    #[test]
    fn test_is_valid_requires_date() {
        assert!(!is_valid_article_url("https://arstechnica.com/tech-policy/some-page/"));
    }

    #[test]
    fn test_is_valid_accepts_dated_article() {
        assert!(is_valid_article_url(
            "https://arstechnica.com/science/2024/03/some-article-slug-here/"
        ));
    }

    #[test]
    fn test_is_valid_rejects_gallery() {
        assert!(!is_valid_article_url(
            "https://arstechnica.com/gallery/2024/03/some-gallery/"
        ));
    }

    #[test]
    fn test_is_valid_rejects_author() {
        assert!(!is_valid_article_url(
            "https://arstechnica.com/author/john-doe/"
        ));
    }
}
