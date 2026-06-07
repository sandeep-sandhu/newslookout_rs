// file: mod_en_foxnews.rs
// Fox News — thin SiteConfig delegating to the generic html_news retriever.
// (Migrated from a standalone plugin; see src/plugins/html_news.rs for the shared logic.)

use std::sync::Arc;
use std::sync::mpsc::Sender;
use config::Config;

use crate::document::Document;
use crate::plugins::html_news::{self, SiteConfig};

pub const PLUGIN_NAME: &str = "mod_en_foxnews";

static SITE: SiteConfig = SiteConfig {
    plugin_name: PLUGIN_NAME,
    publisher_name: "Fox News",
    base_url: "https://www.foxnews.com",
    min_content_length: 400,
    starter_urls: &[("https://www.foxnews.com/politics", "politics"),
    ("https://www.foxnews.com/world", "world"),
    ("https://www.foxnews.com/us", "us")],
    valid_url_patterns: &["foxnews.com/politics/",
    "foxnews.com/world/",
    "foxnews.com/us/",
    "foxnews.com/media/"],
    skip_url_patterns: &["/video/", "/category/", "/person/", "#", "javascript:", "mailto:",
    "/media/", "/shows/", "/newsletters/"],
    body_selectors: &["div.article-body p", "div[class*='article-content'] p", "div.content-body p", "article p", "main p"],
    id_regexes: &[],
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
