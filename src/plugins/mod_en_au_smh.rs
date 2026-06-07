// file: mod_en_au_smh.rs
// Sydney Morning Herald — thin SiteConfig delegating to the generic html_news retriever.
// (Migrated from a standalone plugin; see src/plugins/html_news.rs for the shared logic.)

use std::sync::Arc;
use std::sync::mpsc::Sender;
use config::Config;

use crate::document::Document;
use crate::plugins::html_news::{self, SiteConfig};

pub const PLUGIN_NAME: &str = "mod_en_au_smh";

static SITE: SiteConfig = SiteConfig {
    plugin_name: PLUGIN_NAME,
    publisher_name: "Sydney Morning Herald",
    base_url: "https://www.smh.com.au",
    min_content_length: 400,
    starter_urls: &[("https://www.smh.com.au/national/", "national"),
    ("https://www.smh.com.au/world/", "world"),
    ("https://www.smh.com.au/politics/", "politics"),
    ("https://www.smh.com.au/business/", "business")],
    valid_url_patterns: &["www.smh.com.au/"],
    skip_url_patterns: &["/video/", "#", "javascript:", "mailto:",
    "/quiz/", "/puzzles/", "/culture/", "/lifestyle", "/domain-magazine", "/goodfood",
    "/topic/", "/traveller"],
    body_selectors: &["div[class*='article-body'] p", "div[class*='story-content'] p", "div.article__body p", "article p", "main p"],
    id_regexes: &[],
    min_last_segment_len: 0,
    // SMH articles end with -p<hash>.html (e.g. -p604ix.html); section sub-pages do not
    min_path_depth: 5,
    // SMH article slugs always contain hyphens; section sub-pages like /money/banking do not
    require_slug_hyphen: true,
    // Bypass depth check for real articles that are at depth 4 (e.g. /national/article-p604ix.html)
    article_id_suffix_regex: Some(r"-p[0-9a-z]{4,}\.html$"),
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
    fn test_section_sub_pages_rejected() {
        // Section sub-pages at depth 4 without the -p<hash>.html suffix are filtered
        assert!(!is_valid_article_url(&SITE, "https://www.smh.com.au/world/south-america"));
        assert!(!is_valid_article_url(&SITE, "https://www.smh.com.au/money/banking"));
        assert!(!is_valid_article_url(&SITE, "https://www.smh.com.au/topic/column-8-1r4"));
    }

    #[test]
    fn test_articles_accepted() {
        // Real SMH articles end with -p<hash>.html — id suffix bypasses depth check
        assert!(is_valid_article_url(&SITE,
            "https://www.smh.com.au/national/how-bus-crash-survivor-raised-the-alarm-20260606-p604ix.html"));
        assert!(is_valid_article_url(&SITE,
            "https://www.smh.com.au/politics/federal/title-of-some-article-20260606-p604xy.html"));
    }
}
