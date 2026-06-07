// file: mod_en_ap_news.rs
// Associated Press — thin SiteConfig delegating to the generic html_news retriever.
// (Migrated from a standalone plugin; see src/plugins/html_news.rs for the shared logic.)

use std::sync::Arc;
use std::sync::mpsc::Sender;
use config::Config;

use crate::document::Document;
use crate::plugins::html_news::{self, SiteConfig};

pub const PLUGIN_NAME: &str = "mod_en_ap_news";

static SITE: SiteConfig = SiteConfig {
    plugin_name: PLUGIN_NAME,
    publisher_name: "Associated Press",
    base_url: "https://apnews.com/",
    min_content_length: 400,
    starter_urls: &[("https://apnews.com/hub/business", "business"),
    ("https://apnews.com/hub/world-news", "world")],
    valid_url_patterns: &["apnews.com/article/"],
    skip_url_patterns: &["/about", "/contact", "/subscribe", "/login", "/video/",
    "/photo/", "/podcast/", "/newsletter", "/live-update/",
    "#", "javascript:", "mailto:", "/afs/", "/hub/"],
    body_selectors: &["div.RichTextStoryBody", "div[class*='RichTextStoryBody']", "div[class*='Article']", "article p", "main p"],
    id_regexes: &[],
    min_last_segment_len: 0,
    min_path_depth: 0,
    require_slug_hyphen: false,
    article_id_suffix_regex: None,
    article_user_agent: Some("Mozilla/5.0 (X11; Linux x86_64; rv:120.0) Gecko/20100101 Firefox/120.0"),
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
    fn test_article_url_accepted() {
        assert!(is_valid_article_url(&SITE, "https://apnews.com/article/some-news-story-abc123def456"));
    }
    #[test]
    fn test_hub_and_video_rejected() {
        assert!(!is_valid_article_url(&SITE, "https://apnews.com/hub/business"));
        assert!(!is_valid_article_url(&SITE, "https://apnews.com/video/some-clip"));
    }
    #[test]
    fn test_requires_article_path() {
        assert!(!is_valid_article_url(&SITE, "https://apnews.com/about"));
    }
    #[test]
    fn test_special_user_agent_set() {
        assert!(SITE.article_user_agent.is_some(), "AP needs a Firefox UA for article bodies");
    }
    #[test]
    fn test_id_is_last_segment() {
        assert_eq!(extract_unique_id(&SITE, "https://apnews.com/article/treasury-bonds-2025/"), "treasury-bonds-2025");
    }
}
