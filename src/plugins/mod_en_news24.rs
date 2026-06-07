// file: mod_en_news24.rs
// News24 — thin SiteConfig delegating to the generic html_news retriever.
// (Migrated from a standalone plugin; see src/plugins/html_news.rs for the shared logic.)

use std::sync::Arc;
use std::sync::mpsc::Sender;
use config::Config;

use crate::document::Document;
use crate::plugins::html_news::{self, SiteConfig};

pub const PLUGIN_NAME: &str = "mod_en_news24";

static SITE: SiteConfig = SiteConfig {
    plugin_name: PLUGIN_NAME,
    publisher_name: "News24",
    base_url: "https://www.news24.com/",
    min_content_length: 400,
    starter_urls: &[("https://www.news24.com/news24/", "news"),
    ("https://www.news24.com/fin24/", "finance"),
    ("https://www.news24.com/news24/world/", "world")],
    valid_url_patterns: &["news24.com/news24/",
    "news24.com/fin24/"],
    skip_url_patterns: &["/photos/", "/videos/", "/sport/", "/entertainment/",
    "/lifestyle/", "/motoring/",
    "#", "javascript:", "mailto:",
    "/tag/", "/author/", "/topic/"],
    body_selectors: &["div.article__body p", "div[class*='article__body'] p", "div[class*='article-body'] p", "article p", "div.content p", "main p"],
    id_regexes: &[r"-([A-Za-z0-9]{10,})$"],
    min_last_segment_len: 0,
    min_path_depth: 4,
    require_slug_hyphen: false,
    article_id_suffix_regex: Some(r"-[A-Za-z0-9]{10,}$"),
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

    use crate::plugins::html_news::is_valid_article_url;

    #[test]
    fn test_deep_article_accepted() {
        assert!(is_valid_article_url(&SITE, "https://www.news24.com/news24/world/some-news-story-headline"));
    }
    #[test]
    fn test_id_suffix_bypasses_depth() {
        // valid-pattern url that ends in a long alphanumeric id is accepted
        assert!(is_valid_article_url(&SITE, "https://www.news24.com/news24/story-aB3xY9Kp02"));
    }
    #[test]
    fn test_unknown_section_rejected() {
        assert!(!is_valid_article_url(&SITE, "https://www.news24.com/sport/soccer/match"));
    }
}
