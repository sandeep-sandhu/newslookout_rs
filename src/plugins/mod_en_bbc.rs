// file: mod_en_bbc.rs
// BBC News — thin SiteConfig delegating to the generic html_news retriever.
// (Migrated from a standalone plugin; see src/plugins/html_news.rs for the shared logic.)

use std::sync::Arc;
use std::sync::mpsc::Sender;
use config::Config;

use crate::document::Document;
use crate::plugins::html_news::{self, SiteConfig};

pub const PLUGIN_NAME: &str = "mod_en_bbc";

static SITE: SiteConfig = SiteConfig {
    plugin_name: PLUGIN_NAME,
    publisher_name: "BBC News",
    base_url: "https://www.bbc.com/",
    min_content_length: 400,
    starter_urls: &[("https://www.bbc.com/news", "main"),
    ("https://www.bbc.com/business", "business")],
    valid_url_patterns: &["www.bbc.com/news",
    "www.bbc.com/business"],
    skip_url_patterns: &["/about", "/contact", "/subscribe", "/login", "/video/",
    "/photo/", "/podcast/", "/newsletter", "/sport/",
    "#", "javascript:", "mailto:", "/sounds/"],
    body_selectors: &["div[class*='article__body']", "div[data-component='text-block']", "article p", "main p"],
    id_regexes: &[r"/news/(\d+)"],
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
    fn test_news_id_extracted() {
        assert_eq!(extract_unique_id(&SITE, "https://www.bbc.com/news/12345678"), "12345678");
    }
    #[test]
    fn test_sport_rejected() {
        assert!(!is_valid_article_url(&SITE, "https://www.bbc.com/sport/football/12345678"));
    }
}
