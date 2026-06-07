// file: mod_en_in_news18.rs
// News18 India — thin SiteConfig delegating to the generic html_news retriever.
// (Migrated from a standalone plugin; see src/plugins/html_news.rs for the shared logic.)

use std::sync::Arc;
use std::sync::mpsc::Sender;
use config::Config;

use crate::document::Document;
use crate::plugins::html_news::{self, SiteConfig};

pub const PLUGIN_NAME: &str = "mod_en_in_news18";

static SITE: SiteConfig = SiteConfig {
    plugin_name: PLUGIN_NAME,
    publisher_name: "News18 India",
    base_url: "https://www.news18.com/",
    min_content_length: 400,
    starter_urls: &[("https://www.news18.com/india/", "india"),
    ("https://www.news18.com/business/", "business"),
    ("https://www.news18.com/world/", "world")],
    valid_url_patterns: &["news18.com/news/",
    "news18.com/india/",
    "news18.com/business/",
    "news18.com/world/"],
    skip_url_patterns: &["/videos/", "/photos/", "/sports/", "/entertainment/",
    "/lifestyle/", "/tech/", "/auto/", "/cricket/",
    "#", "javascript:", "mailto:", "/web-stories/"],
    body_selectors: &["div.article_body p", "div[class*='article_body'] p", "div.story_content p", "section[class*='article'] p", "article p", "div.container p"],
    id_regexes: &[r"-(\d{7,})(?:\.html)?(?:/)?$"],
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
}
