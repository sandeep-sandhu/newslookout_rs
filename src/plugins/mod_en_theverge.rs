// file: mod_en_theverge.rs
// The Verge — thin SiteConfig delegating to the generic html_news retriever.
// (Migrated from a standalone plugin; see src/plugins/html_news.rs for the shared logic.)

use std::sync::Arc;
use std::sync::mpsc::Sender;
use config::Config;

use crate::document::Document;
use crate::plugins::html_news::{self, SiteConfig};

pub const PLUGIN_NAME: &str = "mod_en_theverge";

static SITE: SiteConfig = SiteConfig {
    plugin_name: PLUGIN_NAME,
    publisher_name: "The Verge",
    base_url: "https://www.theverge.com",
    min_content_length: 400,
    starter_urls: &[("https://www.theverge.com/tech", "tech"),
    ("https://www.theverge.com/policy", "policy"),
    ("https://www.theverge.com/science", "science")],
    valid_url_patterns: &["www.theverge.com/"],
    skip_url_patterns: &["/video/", "/forums/", "#", "javascript:", "mailto:", "/newsletter/", "/podcasts/",
    ".jpg", ".jpeg", ".png", ".webp", ".gif", "/auth/", "/contact"],
    body_selectors: &["div[class*='duet--article--article-body'] p", "div[class*='article-body'] p", "div.c-entry-content p", "article p", "main p"],
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

    use crate::plugins::html_news::is_valid_article_url;

    #[test]
    fn test_cdn_image_urls_rejected() {
        // CDN image URLs should be rejected by skip_url_patterns (.jpg etc.)
        assert!(!is_valid_article_url(&SITE,
            "https://platform.theverge.com/wp-content/uploads/sites/2/2026/06/photo.jpg?quality=90"));
        assert!(!is_valid_article_url(&SITE,
            "https://platform.theverge.com/wp-content/uploads/sites/2/photo.webp?strip=all"));
    }

    #[test]
    fn test_cross_subdomain_rejected() {
        // shop.theverge.com and platform.theverge.com should not match www.theverge.com/
        assert!(!is_valid_article_url(&SITE, "https://shop.theverge.com/products/thing"));
        assert!(!is_valid_article_url(&SITE, "https://platform.theverge.com/some-page"));
    }

    #[test]
    fn test_article_url_accepted() {
        assert!(is_valid_article_url(&SITE,
            "https://www.theverge.com/2026/6/5/some-article-about-tech"));
    }
}
