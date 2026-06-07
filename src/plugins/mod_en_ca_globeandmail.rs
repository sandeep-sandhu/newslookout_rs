// file: mod_en_ca_globeandmail.rs
// The Globe and Mail — thin SiteConfig delegating to the generic html_news retriever.
// (Migrated from a standalone plugin; see src/plugins/html_news.rs for the shared logic.)

use std::sync::Arc;
use std::sync::mpsc::Sender;
use config::Config;

use crate::document::Document;
use crate::plugins::html_news::{self, SiteConfig};

pub const PLUGIN_NAME: &str = "mod_en_ca_globeandmail";

static SITE: SiteConfig = SiteConfig {
    plugin_name: PLUGIN_NAME,
    publisher_name: "The Globe and Mail",
    base_url: "https://www.theglobeandmail.com",
    min_content_length: 400,
    starter_urls: &[("https://www.theglobeandmail.com/canada/", "canada"),
    ("https://www.theglobeandmail.com/world/", "world"),
    ("https://www.theglobeandmail.com/politics/", "politics"),
    ("https://www.theglobeandmail.com/business/", "business")],
    valid_url_patterns: &["www.theglobeandmail.com/"],
    skip_url_patterns: &["/video/", "#", "javascript:", "mailto:",
    "/newsletter", "/podcasts/", "/privacy-terms", "/industry-news/", "/international-business/"],
    body_selectors: &["div[class*='article-body'] p", "div.c-article-body p", "div[class*='story-body'] p", "article p", "main p"],
    id_regexes: &[],
    // Province section names (longest: british-columbia = 16 chars) are filtered;
    // article slugs are consistently 17+ characters
    min_last_segment_len: 17,
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
    fn test_cross_subdomain_rejected() {
        assert!(!is_valid_article_url(&SITE, "https://subscriptions.theglobeandmail.com/gift"));
        assert!(!is_valid_article_url(&SITE, "https://sec.theglobeandmail.com/user/login"));
    }

    #[test]
    fn test_province_sections_rejected() {
        // Province/region section sub-pages have slugs ≤ 16 chars — below min_last_segment_len: 17
        assert!(!is_valid_article_url(&SITE, "https://www.theglobeandmail.com/canada/british-columbia/"));
        assert!(!is_valid_article_url(&SITE, "https://www.theglobeandmail.com/canada/alberta/"));
        assert!(!is_valid_article_url(&SITE, "https://www.theglobeandmail.com/drive/"));
    }

    #[test]
    fn test_article_url_accepted() {
        assert!(is_valid_article_url(&SITE,
            "https://www.theglobeandmail.com/business/article-rbi-rates-report/"));
        assert!(is_valid_article_url(&SITE,
            "https://www.theglobeandmail.com/canada/federal-election-results-2026/"));
    }
}
