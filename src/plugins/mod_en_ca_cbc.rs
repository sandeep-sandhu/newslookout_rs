// file: mod_en_ca_cbc.rs
// CBC News — thin SiteConfig delegating to the generic html_news retriever.
// (Migrated from a standalone plugin; see src/plugins/html_news.rs for the shared logic.)

use std::sync::Arc;
use std::sync::mpsc::Sender;
use config::Config;

use crate::document::Document;
use crate::plugins::html_news::{self, SiteConfig};

pub const PLUGIN_NAME: &str = "mod_en_ca_cbc";

static SITE: SiteConfig = SiteConfig {
    plugin_name: PLUGIN_NAME,
    publisher_name: "CBC News",
    base_url: "https://www.cbc.ca/news",
    min_content_length: 400,
    starter_urls: &[("https://www.cbc.ca/news/world", "world"),
    ("https://www.cbc.ca/news/politics", "politics"),
    ("https://www.cbc.ca/news/business", "business"),
    ("https://www.cbc.ca/news/canada", "canada")],
    valid_url_patterns: &["www.cbc.ca/news/"],
    skip_url_patterns: &["/video/", "#", "javascript:", "mailto:",
    "/radio/", "/podcasts/", "/player/",
    "/account/", "/search", "/sitemap"],
    body_selectors: &["div[class*='story'] p", "div.detailMainContent p", "div[class*='article-content'] p", "article p", "main p"],
    id_regexes: &[],
    min_last_segment_len: 0,
    min_path_depth: 5,
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
    fn test_section_pages_filtered() {
        // Section pages at depth 4 must be rejected (min_path_depth: 5)
        assert!(!is_valid_article_url(&SITE, "https://www.cbc.ca/news/politics"));
        assert!(!is_valid_article_url(&SITE, "https://www.cbc.ca/news/canada"));
        // Cross-subdomain via valid_url_patterns (gem.cbc.ca → no www.cbc.ca/news/)
        assert!(!is_valid_article_url(&SITE, "https://gem.cbc.ca/media/video.mp4"));
        // Non-news paths filtered by valid_url_patterns (no www.cbc.ca/news/ in path)
        assert!(!is_valid_article_url(&SITE, "https://www.cbc.ca/music"));
        assert!(!is_valid_article_url(&SITE, "https://www.cbc.ca/radio"));
    }

    #[test]
    fn test_article_url_accepted() {
        // Real article: depth 5, correct domain
        assert!(is_valid_article_url(&SITE,
            "https://www.cbc.ca/news/canada/some-story-title-1.6234567"));
        assert!(is_valid_article_url(&SITE,
            "https://www.cbc.ca/news/world/some-article-slug-here"));
    }
}
