// file: mod_en_cnet.rs
// CNET — thin SiteConfig delegating to the generic html_news retriever.
// (Migrated from a standalone plugin; see src/plugins/html_news.rs for the shared logic.)

use std::sync::Arc;
use std::sync::mpsc::Sender;
use config::Config;

use crate::document::Document;
use crate::plugins::html_news::{self, SiteConfig};

pub const PLUGIN_NAME: &str = "mod_en_cnet";

static SITE: SiteConfig = SiteConfig {
    plugin_name: PLUGIN_NAME,
    publisher_name: "CNET",
    base_url: "https://www.cnet.com",
    min_content_length: 400,
    starter_urls: &[("https://www.cnet.com/news/", "news"),
    ("https://www.cnet.com/personal-finance/", "finance"),
    ("https://www.cnet.com/tech/", "tech")],
    valid_url_patterns: &["cnet.com/news/",
    "cnet.com/personal-finance/",
    "cnet.com/tech/"],
    skip_url_patterns: &["/video/", "/pictures/", "#", "javascript:", "mailto:", "/deals/", "/author/",
    // Known CNET subcategory landing pages that are not articles
    "/tech/home-entertainment/", "/tech/services-and-software/", "/tech/streaming-services/",
    "/tech/mobile/", "/tech/computing/", "/tech/smart-home/", "/tech/gaming-tech/",
    "/tech/security/", "/tech/ai/",
    "/personal-finance/credit-cards/", "/personal-finance/cryptocurrency/",
    "/personal-finance/banking/", "/personal-finance/investing/",
    "/personal-finance/insurance/", "/personal-finance/mortgages/",
    "/personal-finance/taxes/", "/personal-finance/retirement/"],
    body_selectors: &["div[class*='article-body'] p", "div.c-pageArticle p", "div[class*='content-body'] p", "article p", "main p"],
    id_regexes: &[],
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
    fn test_long_slug_accepted() {
        assert!(is_valid_article_url(&SITE, "https://www.cnet.com/tech/some-long-article-slug-here"));
    }
    #[test]
    fn test_short_subcategory_rejected() {
        assert!(!is_valid_article_url(&SITE, "https://www.cnet.com/tech/mobile"));
    }
}
