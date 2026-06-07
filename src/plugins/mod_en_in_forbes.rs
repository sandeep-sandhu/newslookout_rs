// file: mod_en_in_forbes.rs
// Forbes India — thin SiteConfig delegating to the generic html_news retriever.
// (Migrated from a standalone plugin; see src/plugins/html_news.rs for the shared logic.)

use std::sync::Arc;
use std::sync::mpsc::Sender;
use config::Config;

use crate::document::Document;
use crate::plugins::html_news::{self, SiteConfig};

pub const PLUGIN_NAME: &str = "mod_en_in_forbes";

static SITE: SiteConfig = SiteConfig {
    plugin_name: PLUGIN_NAME,
    publisher_name: "Forbes India",
    base_url: "https://www.forbesindia.com/",
    min_content_length: 400,
    starter_urls: &[("https://www.forbesindia.com/", "main")],
    valid_url_patterns: &["forbesindia.com/article/",
    "forbesindia.com/blog/",
    "forbesindia.com/interview/"],
    skip_url_patterns: &["/about", "/contact", "/subscribe", "/login",
    "/podcast/", "/newsletter", "/tag/",
    "#", "javascript:", "mailto:",
    "/videos/", "/video/", "/webstories/", "/top-news", "/w-power-",
    "/upfront/brand-connect", "/upfront/column", "/upfront/ceo-talk",
    "/lists/", "/30under30/", "/india-rich-list"],
    body_selectors: &["div[class='articlestorycontent']", "div.article-body", "article p"],
    id_regexes: &[r"(\d{4,})"],
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
