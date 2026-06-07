// file: mod_en_in_hindustan_times.rs
// Hindustan Times — thin SiteConfig delegating to the generic html_news retriever.
// (Migrated from a standalone plugin; see src/plugins/html_news.rs for the shared logic.)

use std::sync::Arc;
use std::sync::mpsc::Sender;
use config::Config;

use crate::document::Document;
use crate::plugins::html_news::{self, SiteConfig};

pub const PLUGIN_NAME: &str = "mod_en_in_hindustan_times";

static SITE: SiteConfig = SiteConfig {
    plugin_name: PLUGIN_NAME,
    publisher_name: "Hindustan Times",
    base_url: "https://www.hindustantimes.com/",
    min_content_length: 400,
    starter_urls: &[("https://www.hindustantimes.com/india-news/", "india"),
    ("https://www.hindustantimes.com/business/", "business"),
    ("https://www.hindustantimes.com/world-news/", "world")],
    valid_url_patterns: &["hindustantimes.com/india-news/",
    "hindustantimes.com/business/",
    "hindustantimes.com/world-news/",
    "hindustantimes.com/cities/"],
    skip_url_patterns: &["/photos/", "/videos/", "/tv/", "/advertisement/",
    "/sports/", "/entertainment/", "/lifestyle/", "/fashion/",
    "/education/", "/astrology/", "/horoscope/",
    "#", "javascript:", "mailto:"],
    body_selectors: &["div.storyDetails p", "div.story-details p", "div[class*='storyDetails'] p", "div[class*='detail'] p", "article p", "div.container p"],
    id_regexes: &[r"-(\d{10,})\.html$"],
    min_last_segment_len: 0,
    min_path_depth: 5,
    require_slug_hyphen: false,
    // HT articles end with -10XXXXXXXXXX.html; section pages don't — bypass depth for these
    article_id_suffix_regex: Some(r"-\d{10,}\.html$"),
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
        for (u, _s) in SITE.starter_urls {
            assert!(u.starts_with("http"), "starter url must be absolute: {}", u);
        }
    }

    use crate::plugins::html_news::is_valid_article_url;

    #[test]
    fn test_section_pages_rejected() {
        // Cities section pages at depth 4 should be filtered
        assert!(!is_valid_article_url(&SITE, "https://www.hindustantimes.com/cities/bengaluru-news"));
        assert!(!is_valid_article_url(&SITE, "https://www.hindustantimes.com/cities/bhopal-news"));
        assert!(!is_valid_article_url(&SITE, "https://www.hindustantimes.com/business/smart-money"));
    }

    #[test]
    fn test_articles_accepted() {
        // Real HT articles end in -10XXXXXXXXXX.html — depth-check is bypassed by id suffix
        assert!(is_valid_article_url(&SITE,
            "https://www.hindustantimes.com/india-news/some-article-101234567890.html"));
        assert!(is_valid_article_url(&SITE,
            "https://www.hindustantimes.com/cities/bengaluru-news/some-article-101234567890.html"));
    }
}
