// file: mod_en_yahoo_news.rs
// Yahoo News — thin SiteConfig delegating to the generic html_news retriever.
// (Migrated from a standalone plugin; see src/plugins/html_news.rs for the shared logic.)

use std::sync::Arc;
use std::sync::mpsc::Sender;
use config::Config;

use crate::document::Document;
use crate::plugins::html_news::{self, SiteConfig};

pub const PLUGIN_NAME: &str = "mod_en_yahoo_news";

static SITE: SiteConfig = SiteConfig {
    plugin_name: PLUGIN_NAME,
    publisher_name: "Yahoo News",
    base_url: "https://news.yahoo.com/",
    min_content_length: 400,
    starter_urls: &[("https://news.yahoo.com/", "main"),
    ("https://finance.yahoo.com/news/", "finance")],
    valid_url_patterns: &["news.yahoo.com/",
    "finance.yahoo.com/news/"],
    skip_url_patterns: &["/video/", "/photos/", "/sports/", "/entertainment/",
    "/lifestyle/", "/tech/", "/science/", "/oddities/",
    "#", "javascript:", "mailto:",
    "/originals/", "/podcasts/", "/newsletters/"],
    body_selectors: &["div.caas-body", "div[class*='article-body']", "div[class*='caas-content']", "article p", "div[class*='body'] p"],
    id_regexes: &[r"-(\d{8,})(?:\.html)?$"],
    min_last_segment_len: 10,
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

    use crate::plugins::html_news::is_valid_article_url;

    #[test]
    fn test_site_config_consistency() {
        assert_eq!(SITE.plugin_name, PLUGIN_NAME);
        assert!(SITE.base_url.starts_with("http"), "base_url must be absolute");
        assert!(!SITE.starter_urls.is_empty(), "need at least one starter url");
        assert!(SITE.min_content_length > 0);
        assert!(!SITE.body_selectors.is_empty(), "need body selectors");
        for (u, _s) in SITE.starter_urls {
            assert!(u.starts_with("http"), "starter url must be absolute: {}", u);
        }
    }

    #[test]
    fn test_pagination_urls_rejected() {
        // /news/2/ through /news/9/ are pagination — short last segment, rejected
        for n in 2..=9 {
            let url = format!("https://news.yahoo.com/news/{}/", n);
            assert!(!is_valid_article_url(&SITE, &url), "pagination {} should be rejected", n);
        }
    }

    #[test]
    fn test_article_url_accepted() {
        // Real article with 8-digit ID
        assert!(is_valid_article_url(&SITE,
            "https://news.yahoo.com/rbi-raises-repo-rate-25-bps-123456789.html"));
        assert!(is_valid_article_url(&SITE,
            "https://finance.yahoo.com/news/some-article-title-123456789.html"));
    }
}
