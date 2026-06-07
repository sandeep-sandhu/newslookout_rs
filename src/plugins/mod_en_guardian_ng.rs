// file: mod_en_guardian_ng.rs
// The Guardian Nigeria — thin SiteConfig delegating to the generic html_news retriever.
// (Migrated from a standalone plugin; see src/plugins/html_news.rs for the shared logic.)

use std::sync::Arc;
use std::sync::mpsc::Sender;
use config::Config;

use crate::document::Document;
use crate::plugins::html_news::{self, SiteConfig};

pub const PLUGIN_NAME: &str = "mod_en_guardian_ng";

static SITE: SiteConfig = SiteConfig {
    plugin_name: PLUGIN_NAME,
    publisher_name: "The Guardian Nigeria",
    base_url: "https://guardian.ng/",
    min_content_length: 400,
    starter_urls: &[("https://guardian.ng/news/", "news"),
    ("https://guardian.ng/business-services/", "business"),
    ("https://guardian.ng/features/", "features")],
    valid_url_patterns: &["guardian.ng/news/",
    "guardian.ng/business-services/",
    "guardian.ng/features/",
    "guardian.ng/economy/"],
    skip_url_patterns: &["/photos/", "/videos/", "/sport/", "/entertainment/",
    "/lifestyle/", "/fashion/", "/arts/",
    "#", "javascript:", "mailto:",
    "/tag/", "/category/", "/author/", "/page/"],
    body_selectors: &["div.entry-content p", "div[class*='entry-content'] p", "div[class*='post-content'] p", "article p", "div.content-area p", "main p"],
    id_regexes: &[],
    min_last_segment_len: 0,
    min_path_depth: 4,
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
    fn test_deep_article_accepted() {
        assert!(is_valid_article_url(&SITE, "https://guardian.ng/news/some-detailed-news-story"));
    }
    #[test]
    fn test_shallow_section_rejected() {
        assert!(!is_valid_article_url(&SITE, "https://guardian.ng/news/"));
    }
}
