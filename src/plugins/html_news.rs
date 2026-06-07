// file: html_news.rs
// Generic, configuration-driven retriever for standard HTML news websites.
//
// The ~47 per-publisher news plugins were previously near-identical copies (~250 lines
// each) differing only in a handful of constants: base URL, starter/listing URLs, URL
// validity patterns, CSS selectors and small ID-extraction tweaks. They are now thin
// `SiteConfig` declarations that delegate to `html_news::run`, eliminating ~12k lines of
// duplication and ensuring a single, well-tested extraction path.
//
// Extraction strategy per article (in order, first success wins):
//   1. `content_extraction::extract_article_content` (readability/model based)
//   2. JSON-LD `articleBody` (when `use_json_ld`)
//   3. Site-specific CSS selectors
// The real publish date is parsed from JSON-LD / meta tags, falling back to "now".

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::Duration;

use chrono::{DateTime, Utc};
use config::Config;
use log::{error, info, warn};
use regex::Regex;
use scraper::{Html, Selector};

use crate::document::Document;
use crate::discovery::{self, RobotsRules};
use crate::network::{http_get, make_http_client, read_network_parameters};
use crate::utils::{clean_text, get_urls_from_database};
use crate::cfg::get_database_filename;
use crate::content_extraction::{extract_article_content, extract_json_ld_article_body};

/// Per-site configuration that fully describes how to crawl and extract from one publisher.
pub struct SiteConfig {
    /// Internal plugin id, e.g. "mod_en_bbc". Used for logging and the completed-urls table.
    pub plugin_name: &'static str,
    /// Human-readable publisher name stored on each Document.
    pub publisher_name: &'static str,
    /// Site root used to resolve protocol-relative / absolute-path links.
    pub base_url: &'static str,
    /// Minimum length (chars) of extracted body text for a document to be accepted.
    pub min_content_length: usize,
    /// Listing/section pages to scrape for article links, paired with a section label.
    pub starter_urls: &'static [(&'static str, &'static str)],
    /// A candidate URL must contain at least one of these substrings (unless empty).
    pub valid_url_patterns: &'static [&'static str],
    /// A candidate URL containing any of these substrings is rejected.
    pub skip_url_patterns: &'static [&'static str],
    /// CSS selectors tried in order to extract article body text.
    pub body_selectors: &'static [&'static str],
    /// Regexes (capture group 1) tried in order to derive a stable unique id from the URL.
    pub id_regexes: &'static [&'static str],
    /// Reject URLs whose final path segment is shorter than this (0 = no check).
    pub min_last_segment_len: usize,
    /// Reject URLs with fewer than this many non-empty '/'-separated segments (0 = no check).
    pub min_path_depth: usize,
    /// Require the final path segment to contain a hyphen (kebab-case article slug).
    pub require_slug_hyphen: bool,
    /// If set and this regex matches the URL, the `min_path_depth` check is bypassed
    /// (used by sites whose articles end in a long alphanumeric id).
    pub article_id_suffix_regex: Option<&'static str>,
    /// Dedicated User-Agent for article fetches (some sites serve a JS shell to the
    /// default UA). Listing pages always use the default client.
    pub article_user_agent: Option<&'static str>,
    /// Whether to try JSON-LD `articleBody` as an extraction fallback.
    pub use_json_ld: bool,
    /// Optional RSS/Atom/sitemap URLs for article discovery (more robust than scraping the
    /// homepage's links). Empty = homepage scraping only.
    pub feed_urls: &'static [&'static str],
    /// Honor robots.txt Disallow rules for this site (recommended).
    pub respect_robots: bool,
}

/// Returns "scheme://host" from a full URL (strips any path component).
/// Used when resolving absolute-path hrefs so base_url's own path is not prepended.
fn scheme_host_of(url: &str) -> String {
    if let Some(scheme_end) = url.find("://") {
        let after = &url[scheme_end + 3..];
        let host_len = after.find('/').unwrap_or(after.len());
        url[..scheme_end + 3 + host_len].to_string()
    } else {
        url.to_string()
    }
}

/// Entry point invoked by each per-site plugin's `run_worker_thread`.
pub fn run(tx: Sender<Document>, app_config: Arc<Config>, site: &SiteConfig) {
    info!("{}: Starting worker thread", site.plugin_name);

    let network_params = read_network_parameters(&app_config);
    let client = make_http_client(&network_params);

    // Optional dedicated article client with a site-specific UA.
    let article_client = match site.article_user_agent {
        Some(ua) => reqwest::blocking::Client::builder()
            .user_agent(ua)
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_else(|_| client.clone()),
        None => client.clone(),
    };

    let database_filename = get_database_filename(&app_config);
    let already_retrieved = get_urls_from_database(&database_filename, site.plugin_name);

    let min_quality: f32 = match app_config.get_float("content_extraction_min_quality") {
        Ok(q) => q as f32,
        Err(_) => 0.1,
    };

    let crawl_interval = Duration::from_secs(network_params.wait_time_min as u64);
    let mut robots_cache: HashMap<String, RobotsRules> = HashMap::new();

    // Build a unified, de-duplicated work list of (section, article_url) from both
    // publisher feeds/sitemaps (preferred) and homepage listing scraping.
    let mut work: Vec<(String, String)> = Vec::new();
    let mut seen_urls: HashSet<String> = HashSet::new();

    for feed_url in site.feed_urls {
        info!("{}: Discovering articles from feed {}", site.plugin_name, feed_url);
        let xml = http_get(&feed_url.to_string(), &client, network_params.retry_times, network_params.wait_time_min);
        if xml.is_empty() {
            warn!("{}: Empty/failed feed {}", site.plugin_name, feed_url);
            continue;
        }
        for url in discovery::extract_links_from_feed(&xml) {
            if is_valid_article_url(site, &url)
                && !already_retrieved.contains(&url)
                && seen_urls.insert(url.clone())
            {
                work.push(("feed".to_string(), url));
            }
        }
    }

    for (starter_url, section_name) in site.starter_urls {
        info!("{}: Fetching listing from {} (section: {})", site.plugin_name, starter_url, section_name);
        let article_urls = get_article_urls_from_listing(
            site, starter_url, &client, &already_retrieved,
            network_params.retry_times, network_params.wait_time_min,
        );
        info!("{}: Found {} article URLs in section {}", site.plugin_name, article_urls.len(), section_name);
        for url in article_urls {
            if seen_urls.insert(url.clone()) {
                work.push((section_name.to_string(), url));
            }
        }
    }

    info!("{}: {} unique article URLs to fetch.", site.plugin_name, work.len());

    for (section_name, article_url) in work {
        // Respect robots.txt.
        if !allowed_by_robots(site, &mut robots_cache, &client, &article_url, network_params.wait_time_min) {
            info!("{}: Skipping (robots.txt disallow) url={}", site.plugin_name, article_url);
            continue;
        }
        // Per-host politeness across all retriever threads sharing this host.
        if let Some(host) = discovery::host_of(&article_url) {
            discovery::throttle_host(&host, crawl_interval);
        }

        match fetch_and_extract_article(
            site, &article_url, &section_name, &article_client, min_quality,
            network_params.retry_times, network_params.wait_time_min,
        ) {
            Some(doc) if doc.text.len() >= site.min_content_length => {
                if let Err(e) = tx.send(doc) {
                    error!("{}: Failed to send document (url={}): {}", site.plugin_name, article_url, e);
                }
            }
            Some(_) => warn!("{}: Skipping article with too little content (url={})", site.plugin_name, article_url),
            None => warn!("{}: Could not extract content (url={})", site.plugin_name, article_url),
        }
    }

    info!("{}: Worker thread completed", site.plugin_name);
}

/// Check robots.txt for the URL's host, fetching and caching the rules on first encounter.
/// Fails open: if robots.txt cannot be fetched or parsed, the URL is allowed.
fn allowed_by_robots(
    site: &SiteConfig,
    cache: &mut HashMap<String, RobotsRules>,
    client: &reqwest::blocking::Client,
    url: &str,
    wait_time: usize,
) -> bool {
    if !site.respect_robots {
        return true;
    }
    let host = match discovery::host_of(url) {
        Some(h) => h,
        None => return true,
    };
    if !cache.contains_key(&host) {
        let robots_url = format!("https://{}/robots.txt", host);
        let body = http_get(&robots_url, client, 1, wait_time);
        cache.insert(host.clone(), RobotsRules::parse(&body));
    }
    cache
        .get(&host)
        .map(|rules| rules.is_allowed(&discovery::path_of(url)))
        .unwrap_or(true)
}

fn get_article_urls_from_listing(
    site: &SiteConfig,
    listing_url: &str,
    client: &reqwest::blocking::Client,
    already_retrieved: &HashSet<String>,
    retry_times: usize,
    wait_time: usize,
) -> Vec<String> {
    let mut urls = Vec::new();

    let html = http_get(&listing_url.to_string(), client, retry_times, wait_time);
    if html.is_empty() {
        error!("{}: Failed to fetch listing page {}", site.plugin_name, listing_url);
        return urls;
    }

    let document = Html::parse_document(&html);
    let link_selector = Selector::parse("a[href]").unwrap_or_else(|_| Selector::parse("a").unwrap());
    let site_host = scheme_host_of(site.base_url);

    for element in document.select(&link_selector) {
        if let Some(href) = element.value().attr("href") {
            let full_url = if href.starts_with("http") {
                href.to_string()
            } else if href.starts_with("//") {
                // Protocol-relative URL — inherit https scheme.
                format!("https:{}", href)
            } else if href.starts_with('/') {
                // Absolute path — join with scheme+host only, not the base_url's path.
                format!("{}{}", site_host, href)
            } else {
                continue;
            };

            if !is_valid_article_url(site, &full_url) {
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

/// Decide whether a discovered URL is an article worth fetching, per the site's rules.
pub fn is_valid_article_url(site: &SiteConfig, url: &str) -> bool {
    if !site.valid_url_patterns.is_empty()
        && !site.valid_url_patterns.iter().any(|p| url.contains(p))
    {
        return false;
    }
    if site.skip_url_patterns.iter().any(|p| url.contains(p)) {
        return false;
    }

    let trimmed = url.trim_end_matches('/');
    let last_seg = trimmed.rsplit('/').next().unwrap_or("");

    if site.min_last_segment_len > 0 && last_seg.len() < site.min_last_segment_len {
        return false;
    }
    if site.require_slug_hyphen && !last_seg.contains('-') {
        return false;
    }

    if site.min_path_depth > 0 {
        let depth = url.split('/').filter(|s| !s.is_empty()).count();
        let id_ok = site
            .article_id_suffix_regex
            .and_then(|p| Regex::new(p).ok())
            .map(|re| re.is_match(trimmed))
            .unwrap_or(false);
        if depth < site.min_path_depth && !id_ok {
            return false;
        }
    }

    true
}

/// Derive a stable unique id from the URL using the site's regexes, then the last path
/// segment, then a hash as a last resort.
pub fn extract_unique_id(site: &SiteConfig, url: &str) -> String {
    for pattern in site.id_regexes {
        if let Ok(re) = Regex::new(pattern) {
            if let Some(caps) = re.captures(url) {
                if let Some(m) = caps.get(1) {
                    return m.as_str().to_string();
                }
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

/// Extract body text using JSON-LD (optional) then site CSS selectors.
pub fn extract_article_body_with_css(site: &SiteConfig, html: &str) -> Option<String> {
    if site.use_json_ld {
        if let Some(body) = extract_json_ld_article_body(html) {
            if body.len() >= site.min_content_length {
                return Some(clean_text(body));
            }
        }
    }

    let document = Html::parse_document(html);
    for selector_str in site.body_selectors {
        if let Ok(selector) = Selector::parse(selector_str) {
            let text: String = document
                .select(&selector)
                .map(|el| el.text().collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            if text.len() >= site.min_content_length {
                return Some(clean_text(text));
            }
        }
    }
    None
}

/// Parse the article's publish date from JSON-LD `datePublished` or common meta tags.
/// Returns (YYYY-MM-DD, unix_timestamp_seconds) when found.
pub fn extract_publish_date(html: &str) -> Option<(String, i64)> {
    // Patterns that capture an ISO-8601-ish date string in group 1.
    let patterns = [
        r#""datePublished"\s*:\s*"([^"]+)""#,
        r#"<meta[^>]+property=["']article:published_time["'][^>]+content=["']([^"']+)["']"#,
        r#"<meta[^>]+content=["']([^"']+)["'][^>]+property=["']article:published_time["']"#,
        r#"<meta[^>]+itemprop=["']datePublished["'][^>]+content=["']([^"']+)["']"#,
        r#"<meta[^>]+name=["']pubdate["'][^>]+content=["']([^"']+)["']"#,
        r#"<time[^>]+datetime=["']([0-9]{4}-[0-9]{2}-[0-9]{2}[^"']*)["']"#,
    ];
    for pat in patterns {
        if let Ok(re) = Regex::new(pat) {
            if let Some(caps) = re.captures(html) {
                if let Some(m) = caps.get(1) {
                    if let Some(parsed) = parse_iso_date(m.as_str()) {
                        return Some(parsed);
                    }
                }
            }
        }
    }
    None
}

/// Parse an ISO-8601 / RFC-3339 date(-time) string into (YYYY-MM-DD, timestamp).
fn parse_iso_date(raw: &str) -> Option<(String, i64)> {
    let s = raw.trim();
    // Full RFC-3339 with offset, e.g. 2024-06-01T10:00:00+05:30 or ...Z
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some((dt.format("%Y-%m-%d").to_string(), dt.timestamp()));
    }
    // Date-only or loosely-formatted: take the leading YYYY-MM-DD if present.
    if s.len() >= 10 {
        let head = &s[..10];
        if let Ok(d) = chrono::NaiveDate::parse_from_str(head, "%Y-%m-%d") {
            let ts = d.and_hms_opt(0, 0, 0).map(|ndt| ndt.and_utc().timestamp()).unwrap_or(0);
            return Some((head.to_string(), ts));
        }
    }
    None
}

fn fetch_and_extract_article(
    site: &SiteConfig,
    url: &str,
    section_name: &str,
    client: &reqwest::blocking::Client,
    min_quality: f32,
    retry_times: usize,
    wait_time: usize,
) -> Option<Document> {
    let html = http_get(&url.to_string(), client, retry_times, wait_time);
    if html.is_empty() {
        error!("{}: Failed to fetch article (url={})", site.plugin_name, url);
        return None;
    }

    let mut doc = Document::default();
    doc.module = site.plugin_name.to_string();
    doc.plugin_name = site.publisher_name.to_string();
    doc.section_name = section_name.to_string();
    doc.url = url.to_string();
    doc.source_name = vec![site.publisher_name.to_string()];
    doc.unique_id = extract_unique_id(site, url);

    // Real publish date when discoverable, else fall back to the crawl date.
    match extract_publish_date(&html) {
        Some((date_str, ts)) => {
            doc.publish_date = date_str;
            doc.publish_date_ms = ts;
        }
        None => {
            doc.publish_date = Utc::now().format("%Y-%m-%d").to_string();
            doc.publish_date_ms = Utc::now().timestamp();
        }
    }

    let html_doc = Html::parse_document(&html);
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

    let text = extract_article_content(&html, min_quality)
        .filter(|t| t.len() >= site.min_content_length)
        .or_else(|| extract_article_body_with_css(site, &html))
        .unwrap_or_default();

    if text.len() < site.min_content_length {
        return None;
    }

    doc.text = text;
    Some(doc)
}

#[cfg(test)]
pub fn test_site() -> SiteConfig {
    SiteConfig {
        plugin_name: "mod_test",
        publisher_name: "Test Publisher",
        base_url: "https://example.com/",
        min_content_length: 400,
        starter_urls: &[("https://example.com/news", "main")],
        valid_url_patterns: &["example.com/news"],
        skip_url_patterns: &["/video/", "/about"],
        body_selectors: &["article p", "main p"],
        id_regexes: &[r"/news/(\d+)"],
        min_last_segment_len: 0,
        min_path_depth: 0,
        require_slug_hyphen: false,
        article_id_suffix_regex: None,
        article_user_agent: None,
        use_json_ld: true,
        feed_urls: &[],
        respect_robots: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scheme_host_strips_path() {
        assert_eq!(scheme_host_of("https://www.cbc.ca/news"), "https://www.cbc.ca");
        assert_eq!(scheme_host_of("https://www.abc.net.au/news/world"), "https://www.abc.net.au");
        assert_eq!(scheme_host_of("https://www.bbc.com"), "https://www.bbc.com");
        assert_eq!(scheme_host_of("http://example.com:8080/foo"), "http://example.com:8080");
    }

    #[test]
    fn test_absolute_path_uses_host_only() {
        // If base_url has a path (e.g. /news), absolute hrefs like /music should resolve
        // to https://host/music — NOT https://host/news/music.
        let host = scheme_host_of("https://www.cbc.ca/news");
        assert_eq!(
            format!("{}{}", host, "/music"),
            "https://www.cbc.ca/music"
        );
        // And /news/canada/story resolves correctly too.
        assert_eq!(
            format!("{}{}", host, "/news/canada/story"),
            "https://www.cbc.ca/news/canada/story"
        );
    }

    #[test]
    fn test_protocol_relative_gets_https() {
        // //gem.cbc.ca should become https://gem.cbc.ca (then filtered by valid_url_patterns)
        let resolved = format!("https:{}", "//gem.cbc.ca/media/video.mp4");
        assert_eq!(resolved, "https://gem.cbc.ca/media/video.mp4");
        // That URL won't pass CBC's valid_url_pattern "cbc.ca/news/"
        let s = SiteConfig {
            valid_url_patterns: &["cbc.ca/news/"],
            skip_url_patterns: &[],
            ..test_site()
        };
        assert!(!is_valid_article_url(&s, &resolved));
    }

    #[test]
    fn test_valid_pattern_required() {
        let s = test_site();
        assert!(is_valid_article_url(&s, "https://example.com/news/12345678-some-story"));
        assert!(!is_valid_article_url(&s, "https://example.com/sport/12345678"));
    }

    #[test]
    fn test_skip_pattern_rejected() {
        let s = test_site();
        assert!(!is_valid_article_url(&s, "https://example.com/news/video/clip"));
        assert!(!is_valid_article_url(&s, "https://example.com/news/about"));
    }

    #[test]
    fn test_min_last_segment_len() {
        let s = SiteConfig { min_last_segment_len: 10, ..test_site() };
        assert!(!is_valid_article_url(&s, "https://example.com/news/tech"));
        assert!(is_valid_article_url(&s, "https://example.com/news/a-long-descriptive-slug"));
    }

    #[test]
    fn test_min_path_depth() {
        let s = SiteConfig { min_path_depth: 4, ..test_site() };
        // depth counts scheme + host + path segments
        assert!(!is_valid_article_url(&s, "https://example.com/news")); // depth 3
        assert!(is_valid_article_url(&s, "https://example.com/news/world/story")); // depth 5
    }

    #[test]
    fn test_article_id_suffix_bypasses_depth() {
        let s = SiteConfig {
            min_path_depth: 4,
            article_id_suffix_regex: Some(r"-[A-Za-z0-9]{10,}$"),
            ..test_site()
        };
        // Shallow path but ends with long id -> accepted
        assert!(is_valid_article_url(&s, "https://example.com/news/story-aB3xY9Kp02"));
    }

    #[test]
    fn test_require_slug_hyphen() {
        let s = SiteConfig { require_slug_hyphen: true, ..test_site() };
        assert!(!is_valid_article_url(&s, "https://example.com/news/storyword"));
        assert!(is_valid_article_url(&s, "https://example.com/news/two-words"));
    }

    #[test]
    fn test_extract_unique_id_regex() {
        let s = test_site();
        assert_eq!(extract_unique_id(&s, "https://example.com/news/12345678"), "12345678");
    }

    #[test]
    fn test_extract_unique_id_last_segment_fallback() {
        let s = SiteConfig { id_regexes: &[], ..test_site() };
        assert_eq!(extract_unique_id(&s, "https://example.com/news/my-story/"), "my-story");
    }

    #[test]
    fn test_body_extraction_css() {
        let s = test_site();
        let para = "This is a substantial paragraph of article text used for testing. ";
        let html = format!(
            "<html><body><article>{}</article></body></html>",
            (0..10).map(|i| format!("<p>{} {}</p>", para, i)).collect::<Vec<_>>().join("")
        );
        let out = extract_article_body_with_css(&s, &html);
        assert!(out.is_some());
        assert!(out.unwrap().len() >= s.min_content_length);
    }

    #[test]
    fn test_body_extraction_json_ld() {
        let s = test_site();
        let body = "Full structured article body content extracted from JSON-LD data. ".repeat(8);
        let html = format!(
            r#"<html><head><script type="application/ld+json">{{"@type":"NewsArticle","articleBody":"{}"}}</script></head><body></body></html>"#,
            body
        );
        assert!(extract_article_body_with_css(&s, &html).is_some());
    }

    #[test]
    fn test_short_body_returns_none() {
        let s = test_site();
        let html = "<html><body><article><p>Too short.</p></article></body></html>";
        assert!(extract_article_body_with_css(&s, html).is_none());
    }

    #[test]
    fn test_publish_date_from_json_ld() {
        let html = r#"<script type="application/ld+json">{"@type":"NewsArticle","datePublished":"2024-06-01T10:30:00Z"}</script>"#;
        let (d, ts) = extract_publish_date(html).expect("should find date");
        assert_eq!(d, "2024-06-01");
        assert!(ts > 0);
    }

    #[test]
    fn test_publish_date_from_meta() {
        let html = r#"<meta property="article:published_time" content="2023-12-25T08:00:00+05:30"/>"#;
        let (d, _) = extract_publish_date(html).expect("should find date");
        assert_eq!(d, "2023-12-25");
    }

    #[test]
    fn test_publish_date_date_only() {
        let html = r#"<meta itemprop="datePublished" content="2022-01-15">"#;
        let (d, _) = extract_publish_date(html).expect("should find date");
        assert_eq!(d, "2022-01-15");
    }

    #[test]
    fn test_publish_date_absent() {
        assert!(extract_publish_date("<html><body>no date here</body></html>").is_none());
    }
}
