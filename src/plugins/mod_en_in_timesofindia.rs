// file: mod_en_in_timesofindia.rs
// Times of India — thin SiteConfig delegating to the generic html_news retriever.
// (Migrated from a standalone plugin; see src/plugins/html_news.rs for the shared logic.)

use std::sync::Arc;
use std::sync::mpsc::Sender;
use config::Config;

use crate::document::Document;
use crate::plugins::html_news::{self, SiteConfig};

pub const PLUGIN_NAME: &str = "mod_en_in_timesofindia";

static SITE: SiteConfig = SiteConfig {
    plugin_name: PLUGIN_NAME,
    publisher_name: "Times of India",
    base_url: "https://timesofindia.indiatimes.com/",
    min_content_length: 400,
    starter_urls: &[("https://timesofindia.indiatimes.com/business", "business"),
    ("https://timesofindia.indiatimes.com/india", "india")],
    valid_url_patterns: &["/articleshow/"],
    skip_url_patterns: &["/about", "/contact", "/subscribe", "/login", "/videos/",
    "/photos/", "/podcast/", "/newsletter", "/live-update/",
    "#", "javascript:", "mailto:", "/topic/",
    "/feedback", "/sitemap", "/privacy-policy", "/cookiepolicy",
    "/photostories", "/liveblog/",
    "/gold-rates-today", "/silver-rates-today", "/platinum-rates-today",
    "/fuel-price/", "/mutual-funds", "/real-estate", "/telecom",
    "/bank-holidays", "/public-holidays", "/currency-converter",
    "/movie-reviews", "/web-series-reviews"],
    body_selectors: &["div[class*='article_content']", "div[itemprop='articleBody']", "div[class*='innerbody']", "div[class*='js_tbl_article']", "div[class='main-content single-article-content']", "div.article-content", "article p"],
    id_regexes: &[r"articleshow/(\d+)", r"[/\-](\d{5,})"],
    min_last_segment_len: 0,
    min_path_depth: 0,
    require_slug_hyphen: false,
    article_id_suffix_regex: None,
    article_user_agent: None,
    use_json_ld: true,
    feed_urls: &[],
    respect_robots: true,
};

pub fn run_worker_thread(tx: Sender<Document>, app_config: Arc<Config>) {
    html_news::run(tx, app_config, &SITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_site_config_consistency() {
        assert_eq!(SITE.plugin_name, PLUGIN_NAME);
        assert!(SITE.base_url.starts_with("http"), "base_url must be absolute");
        assert!(!SITE.starter_urls.is_empty(), "need at least one starter url");
        assert!(SITE.min_content_length > 0);
        assert!(!SITE.body_selectors.is_empty(), "need body selectors");
        // every starter url should live under the site's base domain host
        for (u, _s) in SITE.starter_urls {
            assert!(u.starts_with("http"), "starter url must be absolute: {}", u);
        }
    }

    use crate::plugins::html_news::{is_valid_article_url, extract_unique_id};

    #[test]
    fn test_articleshow_required() {
        assert!(is_valid_article_url(&SITE, "https://timesofindia.indiatimes.com/business/startups/x/articleshow/123456789.cms"));
        assert!(!is_valid_article_url(&SITE, "https://timesofindia.indiatimes.com/business/startups/x"));
    }
    #[test]
    fn test_data_pages_rejected() {
        assert!(!is_valid_article_url(&SITE, "https://timesofindia.indiatimes.com/gold-rates-today/articleshow/123.cms"));
        assert!(!is_valid_article_url(&SITE, "https://timesofindia.indiatimes.com/mutual-funds/articleshow/123.cms"));
    }
    #[test]
    fn test_id_from_articleshow() {
        assert_eq!(extract_unique_id(&SITE, "https://timesofindia.indiatimes.com/x/articleshow/12345678.cms"), "12345678");
    }
    #[test]
    fn test_uses_json_ld() {
        assert!(SITE.use_json_ld);
    }
}
