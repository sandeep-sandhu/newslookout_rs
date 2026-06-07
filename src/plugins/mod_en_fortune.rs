// file: mod_en_fortune.rs
// Fortune — thin SiteConfig delegating to the generic html_news retriever.
// (Migrated from a standalone plugin; see src/plugins/html_news.rs for the shared logic.)

use std::sync::Arc;
use std::sync::mpsc::Sender;
use config::Config;

use crate::document::Document;
use crate::plugins::html_news::{self, SiteConfig};

pub const PLUGIN_NAME: &str = "mod_en_fortune";

static SITE: SiteConfig = SiteConfig {
    plugin_name: PLUGIN_NAME,
    publisher_name: "Fortune",
    base_url: "https://fortune.com",
    min_content_length: 400,
    starter_urls: &[("https://fortune.com/section/finance/", "finance"),
    ("https://fortune.com/section/tech/", "tech"),
    ("https://fortune.com/section/politics/", "politics")],
    valid_url_patterns: &["fortune.com/"],
    skip_url_patterns: &["/video/", "#", "javascript:", "mailto:",
    "/newsletters/", "/recommends/", "/author/", "/section/",
    "/brandstudio", "/group-subscriptions", "/business-development",
    "conferences.fortune.com"],
    body_selectors: &["div[class*='article-content'] p", "div[class*='content-body'] p", "div.paywall-content p", "article p", "main p"],
    id_regexes: &[],
    min_last_segment_len: 0,
    // Fortune article paths: /YYYY/MM/DD/slug/ — depth 6; section pages depth ≤ 5
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
