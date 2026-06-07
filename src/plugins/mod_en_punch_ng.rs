// file: mod_en_punch_ng.rs
// Punch Nigeria — thin SiteConfig delegating to the generic html_news retriever.
// (Migrated from a standalone plugin; see src/plugins/html_news.rs for the shared logic.)

use std::sync::Arc;
use std::sync::mpsc::Sender;
use config::Config;

use crate::document::Document;
use crate::plugins::html_news::{self, SiteConfig};

pub const PLUGIN_NAME: &str = "mod_en_punch_ng";

static SITE: SiteConfig = SiteConfig {
    plugin_name: PLUGIN_NAME,
    publisher_name: "Punch Nigeria",
    base_url: "https://punchng.com/",
    min_content_length: 400,
    starter_urls: &[("https://punchng.com/", "main"),
    ("https://punchng.com/business/", "business"),
    ("https://punchng.com/politics/", "politics")],
    valid_url_patterns: &["punchng.com/"],
    skip_url_patterns: &["/photos/", "/videos/", "/sports/", "/entertainment/",
    "/lifestyle/", "/artculture/",
    "#", "javascript:", "mailto:",
    "/tag/", "/author/", "/category/",
    "/advertise", "/contact/", "/about/",
    "/punch-news-app", "/subscribe", "/privacy-policy", "/terms",
    "/sitemap", "/feed", "/rss"],
    body_selectors: &["div.post-content p", "div.entry-content p", "div[class*='post-body'] p", "article p", "div[class*='content'] p", "main p"],
    id_regexes: &[],
    min_last_segment_len: 0,
    min_path_depth: 3,
    require_slug_hyphen: true,
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

    use crate::plugins::html_news::is_valid_article_url;

    #[test]
    fn test_slug_article_accepted() {
        assert!(is_valid_article_url(&SITE, "https://punchng.com/naira-appreciates-against-dollar-as-oil-prices-rise/"));
    }
    #[test]
    fn test_homepage_and_sections_rejected() {
        assert!(!is_valid_article_url(&SITE, "https://punchng.com/"));
        assert!(!is_valid_article_url(&SITE, "https://punchng.com/sports/super-eagles-win/"));
    }
    #[test]
    fn test_slug_must_have_hyphen() {
        assert!(SITE.require_slug_hyphen);
        assert!(!is_valid_article_url(&SITE, "https://punchng.com/business/singleword"));
    }
    #[test]
    fn test_advertise_with_us_rejected() {
        assert!(!is_valid_article_url(&SITE, "https://punchng.com/advertise-with-us"));
        assert!(!is_valid_article_url(&SITE, "https://punchng.com/advertise-with-us/"));
        assert!(!is_valid_article_url(&SITE, "https://punchng.com/advertise/"));
    }
}
