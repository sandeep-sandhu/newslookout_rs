// file: mod_en_chicago_tribune.rs
// Chicago Tribune — thin SiteConfig delegating to the generic html_news retriever.
// (Migrated from a standalone plugin; see src/plugins/html_news.rs for the shared logic.)

use std::sync::Arc;
use std::sync::mpsc::Sender;
use config::Config;

use crate::document::Document;
use crate::plugins::html_news::{self, SiteConfig};

pub const PLUGIN_NAME: &str = "mod_en_chicago_tribune";

static SITE: SiteConfig = SiteConfig {
    plugin_name: PLUGIN_NAME,
    publisher_name: "Chicago Tribune",
    base_url: "https://www.chicagotribune.com",
    min_content_length: 400,
    starter_urls: &[("https://www.chicagotribune.com/news/", "news"),
    ("https://www.chicagotribune.com/business/", "business"),
    ("https://www.chicagotribune.com/politics/", "politics")],
    valid_url_patterns: &["www.chicagotribune.com/"],
    skip_url_patterns: &["/video/", "/gallery/", "#", "javascript:", "mailto:",
    "/sports/", "/entertainment/", "/login", "/logout", "/subscribe", "/page/"],
    body_selectors: &["div[class*='article-body'] p", "div[class*='story-body'] p", "div.article-content p", "article p", "main p"],
    id_regexes: &[],
    min_last_segment_len: 0,
    // CT articles use date-format paths: /YYYY/MM/DD/slug/ — depth 6; section pages are depth ≤ 5
    min_path_depth: 6,
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

    use crate::plugins::html_news::is_valid_article_url;

    #[test]
    fn test_site_config_consistency() {
        assert_eq!(SITE.plugin_name, PLUGIN_NAME);
        assert!(SITE.base_url.starts_with("http"), "base_url must be absolute");
        assert!(!SITE.starter_urls.is_empty(), "need at least one starter url");
        assert!(SITE.min_content_length > 0);
        assert!(!SITE.body_selectors.is_empty(), "need body selectors");
        for (u, _s) in SITE.starter_urls {
            assert!(u.starts_with("http"), "starter url must be absolute: {}", u);
        }
    }

    #[test]
    fn test_cross_subdomain_rejected() {
        // myaccount and placeanad subdomains should be rejected (no www.chicagotribune.com/)
        assert!(!is_valid_article_url(&SITE, "https://myaccount.chicagotribune.com/"));
        assert!(!is_valid_article_url(&SITE, "https://placeanad.chicagotribune.com/whos-who"));
    }

    #[test]
    fn test_section_pages_rejected() {
        // Section pages at depth ≤ 5 should be filtered
        assert!(!is_valid_article_url(&SITE, "https://www.chicagotribune.com/news/"));
        assert!(!is_valid_article_url(&SITE, "https://www.chicagotribune.com/news/crime-public-safety/"));
        assert!(!is_valid_article_url(&SITE, "https://www.chicagotribune.com/news/politics/elections/"));
        assert!(!is_valid_article_url(&SITE, "https://www.chicagotribune.com/login"));
    }

    #[test]
    fn test_article_url_accepted() {
        // CT articles use YYYY/MM/DD/slug format — depth 6
        assert!(is_valid_article_url(&SITE,
            "https://www.chicagotribune.com/2026/06/05/some-article-title/"));
        assert!(is_valid_article_url(&SITE,
            "https://www.chicagotribune.com/2024/01/01/ask-amy/"));
    }
}
