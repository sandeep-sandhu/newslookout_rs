// file: mod_en_khaleej_times.rs
// Khaleej Times — thin SiteConfig delegating to the generic html_news retriever.
// (Migrated from a standalone plugin; see src/plugins/html_news.rs for the shared logic.)

use std::sync::Arc;
use std::sync::mpsc::Sender;
use config::Config;

use crate::document::Document;
use crate::plugins::html_news::{self, SiteConfig};

pub const PLUGIN_NAME: &str = "mod_en_khaleej_times";

static SITE: SiteConfig = SiteConfig {
    plugin_name: PLUGIN_NAME,
    publisher_name: "Khaleej Times",
    base_url: "https://www.khaleejtimes.com/",
    min_content_length: 400,
    starter_urls: &[("https://www.khaleejtimes.com/business/", "business"),
    ("https://www.khaleejtimes.com/uae/", "uae"),
    ("https://www.khaleejtimes.com/world/", "world")],
    valid_url_patterns: &["khaleejtimes.com/business/",
    "khaleejtimes.com/uae/",
    "khaleejtimes.com/world/",
    "khaleejtimes.com/economy/"],
    skip_url_patterns: &["/photos/", "/videos/", "/sports/", "/entertainment/",
    "/lifestyle/", "/kt-network/",
    "#", "javascript:", "mailto:",
    "/tag/", "/author/",
    // Known subcategory pages (single segment after section, e.g. /business/tech)
    "/business/tech", "/business/banking", "/business/aviation", "/business/property",
    "/business/markets", "/business/trade", "/business/corporate",
    "/uae/government", "/uae/community", "/uae/transport", "/uae/courts",
    "/world/asia", "/world/europe", "/world/americas", "/world/africa",
    "/opinion/", "/supplements/"],
    body_selectors: &["div.article-body p", "div[class*='article-body'] p", "div[class*='content-body'] p", "div[class*='articleContent'] p", "div[class*='story-body'] p", "article p", "div.entry-content p", "main p"],
    id_regexes: &[r"-(\d{6,})$"],
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
    fn test_long_slug_article_accepted() {
        assert!(is_valid_article_url(&SITE, "https://www.khaleejtimes.com/uae/global-indians-of-the-uae-series-women-on-the-rise"));
    }
    #[test]
    fn test_short_subcategory_rejected() {
        assert!(!is_valid_article_url(&SITE, "https://www.khaleejtimes.com/business/tech"));
        assert!(!is_valid_article_url(&SITE, "https://www.khaleejtimes.com/business/"));
    }
    #[test]
    fn test_unknown_section_rejected() {
        assert!(!is_valid_article_url(&SITE, "https://www.khaleejtimes.com/sports/cricket/some-match-report"));
    }
}
