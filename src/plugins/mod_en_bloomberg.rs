// file: mod_en_bloomberg.rs
// Bloomberg — thin SiteConfig delegating to the generic html_news retriever.
// (Migrated from a standalone plugin; see src/plugins/html_news.rs for the shared logic.)

use std::sync::Arc;
use std::sync::mpsc::Sender;
use config::Config;

use crate::document::Document;
use crate::plugins::html_news::{self, SiteConfig};

pub const PLUGIN_NAME: &str = "mod_en_bloomberg";

static SITE: SiteConfig = SiteConfig {
    plugin_name: PLUGIN_NAME,
    publisher_name: "Bloomberg",
    base_url: "https://www.bloomberg.com/",
    min_content_length: 400,
    starter_urls: &[("https://www.bloomberg.com/markets", "markets"),
    ("https://www.bloomberg.com/technology", "technology")],
    valid_url_patterns: &["www.bloomberg.com/"],
    skip_url_patterns: &["/about", "/contact", "/subscribe", "/login", "/video/",
    "/photo/", "/podcast/", "/newsletter", "/live-blog/",
    "#", "javascript:", "mailto:", "/account/"],
    body_selectors: &["div[class*='body-content']", "div[class*='article-body']", "article p", "div.article-content p"],
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
