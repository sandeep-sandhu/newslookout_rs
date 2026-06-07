// file: mod_en_guardian.rs
// The Guardian — thin SiteConfig delegating to the generic html_news retriever.
// (Migrated from a standalone plugin; see src/plugins/html_news.rs for the shared logic.)

use std::sync::Arc;
use std::sync::mpsc::Sender;
use config::Config;

use crate::document::Document;
use crate::plugins::html_news::{self, SiteConfig};

pub const PLUGIN_NAME: &str = "mod_en_guardian";

static SITE: SiteConfig = SiteConfig {
    plugin_name: PLUGIN_NAME,
    publisher_name: "The Guardian",
    base_url: "https://www.theguardian.com/",
    min_content_length: 400,
    starter_urls: &[("https://www.theguardian.com/world", "world"),
    ("https://www.theguardian.com/business", "business")],
    valid_url_patterns: &["www.theguardian.com/"],
    skip_url_patterns: &["/about", "/contact", "/subscribe", "/login", "/video/",
    "/picture/", "/podcast/", "/newsletter", "/sport/",
    "#", "javascript:", "mailto:", "/crosswords/",
    "/preference/", "/index/", "/profile/", "/tone/",
    "/theguardian/series/", "/global/", "/artanddesign",
    "/books", "/travel", "/stage", "/lifeandstyle",
    "/gallery/", "/audio/", "/info/", "/sign-up"],
    body_selectors: &["div[class*='article-body']", "div[itemprop='articleBody']", "div[class*='content__article']", "article p"],
    id_regexes: &[],
    min_last_segment_len: 0,
    // Guardian articles: /section/YYYY/mon/DD/slug — depth 6; tag/section pages depth ≤ 5
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
    fn test_non_article_types_rejected() {
        assert!(!is_valid_article_url(&SITE,
            "https://www.theguardian.com/money/gallery/2026/jun/05/homes-for-sale"));
        assert!(!is_valid_article_url(&SITE,
            "https://www.theguardian.com/world/audio/2026/jun/01/podcast-episode"));
        assert!(!is_valid_article_url(&SITE,
            "https://www.theguardian.com/info/2017/may/16/guardian-business-today-sign-up"));
    }

    #[test]
    fn test_homepage_section_and_tag_rejected() {
        // Homepage (depth 2), top sections (depth 3), tag pages (depth 4) all < min_path_depth: 6
        assert!(!is_valid_article_url(&SITE, "https://www.theguardian.com/"));
        assert!(!is_valid_article_url(&SITE, "https://www.theguardian.com/world"));
        assert!(!is_valid_article_url(&SITE, "https://www.theguardian.com/us-news/donaldtrump"));
        assert!(!is_valid_article_url(&SITE, "https://www.theguardian.com/us-news/trump-administration"));
    }

    #[test]
    fn test_article_url_accepted() {
        assert!(is_valid_article_url(&SITE,
            "https://www.theguardian.com/world/2026/jun/05/some-news-story"));
        assert!(is_valid_article_url(&SITE,
            "https://www.theguardian.com/business/2026/jun/05/markets-update-report"));
    }
}
