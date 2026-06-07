// file: discovery.rs
// Article discovery and crawl-politeness helpers used by the generic news retriever.
//
//  - Feed/sitemap parsing: extract article URLs from RSS, Atom and XML sitemaps. These are
//    far more robust than scraping a homepage's <a href> links (which depend on layout and
//    JS), and they are the publisher-sanctioned discovery surface.
//  - robots.txt: a minimal parser/checker so we don't fetch disallowed paths.
//  - Per-host rate limiting: a process-wide minimum interval between requests to the same
//    host, so parallel plugins hitting the same CDN stay polite.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use regex::Regex;

/// Extract the host (lowercased) from an absolute URL, without pulling in a URL crate.
pub fn host_of(url: &str) -> Option<String> {
    let after_scheme = url.split("://").nth(1)?;
    let host = after_scheme.split(['/', '?', '#']).next()?;
    // strip userinfo@ and :port
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host);
    if host.is_empty() {
        None
    } else {
        Some(host.to_lowercase())
    }
}

/// Parse article links out of an RSS/Atom feed or an XML sitemap.
/// Handles: sitemap `<loc>`, RSS `<link>text</link>`, and Atom `<link href="…"/>`.
pub fn extract_links_from_feed(xml: &str) -> Vec<String> {
    let mut links = Vec::new();

    // XML sitemap and RSS GUIDs/links use <loc>…</loc>.
    if let Ok(re) = Regex::new(r"(?is)<loc>\s*(.*?)\s*</loc>") {
        for caps in re.captures_iter(xml) {
            if let Some(m) = caps.get(1) {
                links.push(clean_xml_text(m.as_str()));
            }
        }
    }

    // RSS 2.0 item links: <link>https://…</link>
    if let Ok(re) = Regex::new(r"(?is)<link>\s*(https?://.*?)\s*</link>") {
        for caps in re.captures_iter(xml) {
            if let Some(m) = caps.get(1) {
                links.push(clean_xml_text(m.as_str()));
            }
        }
    }

    // Atom links: <link ... href="https://…" ... rel="alternate"?>
    if let Ok(re) = Regex::new(r#"(?is)<link[^>]+href=["'](https?://[^"']+)["']"#) {
        for caps in re.captures_iter(xml) {
            if let Some(m) = caps.get(1) {
                links.push(clean_xml_text(m.as_str()));
            }
        }
    }

    links.sort();
    links.dedup();
    links
}

fn clean_xml_text(s: &str) -> String {
    s.trim()
        .trim_start_matches("<![CDATA[")
        .trim_end_matches("]]>")
        .trim()
        .to_string()
}

/// A minimal robots.txt rule set for the `*` user-agent (plus our own UA if matched).
#[derive(Debug, Default, Clone)]
pub struct RobotsRules {
    disallow: Vec<String>,
    allow: Vec<String>,
}

impl RobotsRules {
    /// Parse robots.txt content, collecting Allow/Disallow paths that apply to the `*`
    /// user-agent group. Unknown directives are ignored.
    pub fn parse(content: &str) -> RobotsRules {
        let mut rules = RobotsRules::default();
        let mut applies = false;

        for raw_line in content.lines() {
            // strip comments
            let line = raw_line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (key, value) = match line.split_once(':') {
                Some((k, v)) => (k.trim().to_lowercase(), v.trim().to_string()),
                None => continue,
            };
            match key.as_str() {
                "user-agent" => {
                    // A group applies if it targets all agents.
                    applies = value == "*";
                }
                "disallow" if applies => {
                    if !value.is_empty() {
                        rules.disallow.push(value);
                    }
                }
                "allow" if applies => {
                    if !value.is_empty() {
                        rules.allow.push(value);
                    }
                }
                _ => {}
            }
        }
        rules
    }

    /// Returns true if `path` is permitted. Longest-match wins between Allow and Disallow,
    /// matching the de-facto robots.txt precedence used by major crawlers.
    pub fn is_allowed(&self, path: &str) -> bool {
        let longest_allow = self
            .allow
            .iter()
            .filter(|p| path.starts_with(p.as_str()))
            .map(|p| p.len())
            .max();
        let longest_disallow = self
            .disallow
            .iter()
            .filter(|p| path.starts_with(p.as_str()))
            .map(|p| p.len())
            .max();

        match (longest_allow, longest_disallow) {
            (None, None) => true,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (Some(a), Some(d)) => a >= d, // tie or longer allow -> allowed
        }
    }
}

/// Path component of an absolute URL (everything from the first '/' after the host,
/// including query). Defaults to "/".
pub fn path_of(url: &str) -> String {
    if let Some(after_scheme) = url.split("://").nth(1) {
        if let Some(slash_idx) = after_scheme.find('/') {
            return after_scheme[slash_idx..].to_string();
        }
    }
    "/".to_string()
}

// ── Per-host rate limiting ──────────────────────────────────────────────────────────────

fn last_request_times() -> &'static Mutex<HashMap<String, Instant>> {
    static TIMES: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    TIMES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Block until at least `min_interval` has elapsed since the last request to `host`, then
/// record the current time as the latest request to that host. Process-wide, so parallel
/// retriever threads sharing a host are serialized to a polite cadence.
pub fn throttle_host(host: &str, min_interval: Duration) {
    // Compute how long to wait while holding the lock briefly, then sleep outside the lock.
    let wait = {
        let mut map = match last_request_times().lock() {
            Ok(m) => m,
            Err(poisoned) => poisoned.into_inner(),
        };
        let now = Instant::now();
        let wait = match map.get(host) {
            Some(&last) => {
                let elapsed = now.duration_since(last);
                if elapsed < min_interval {
                    Some(min_interval - elapsed)
                } else {
                    None
                }
            }
            None => None,
        };
        // Reserve the slot now (last = now + wait) so concurrent callers stagger.
        let scheduled = now + wait.unwrap_or_default();
        map.insert(host.to_string(), scheduled);
        wait
    };
    if let Some(w) = wait {
        std::thread::sleep(w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_of() {
        assert_eq!(host_of("https://www.bbc.com/news/123").as_deref(), Some("www.bbc.com"));
        assert_eq!(host_of("http://Example.COM:8080/a?b=1").as_deref(), Some("example.com"));
        assert_eq!(host_of("https://user@host.io/x").as_deref(), Some("host.io"));
        assert_eq!(host_of("not a url"), None);
    }

    #[test]
    fn test_path_of() {
        assert_eq!(path_of("https://x.com/a/b?c=1"), "/a/b?c=1");
        assert_eq!(path_of("https://x.com"), "/");
    }

    #[test]
    fn test_sitemap_loc_extraction() {
        let xml = r#"<urlset><url><loc>https://x.com/a</loc></url><url><loc> https://x.com/b </loc></url></urlset>"#;
        let links = extract_links_from_feed(xml);
        assert!(links.contains(&"https://x.com/a".to_string()));
        assert!(links.contains(&"https://x.com/b".to_string()));
    }

    #[test]
    fn test_rss_link_extraction() {
        let xml = r#"<rss><channel><item><link>https://x.com/story-1</link></item>
                     <item><link>https://x.com/story-2</link></item></channel></rss>"#;
        let links = extract_links_from_feed(xml);
        assert!(links.contains(&"https://x.com/story-1".to_string()));
        assert!(links.contains(&"https://x.com/story-2".to_string()));
    }

    #[test]
    fn test_atom_href_extraction() {
        let xml = r#"<feed><entry><link href="https://x.com/atom-1" rel="alternate"/></entry></feed>"#;
        let links = extract_links_from_feed(xml);
        assert!(links.contains(&"https://x.com/atom-1".to_string()));
    }

    #[test]
    fn test_cdata_cleaned() {
        let xml = r#"<loc><![CDATA[https://x.com/cdata]]></loc>"#;
        let links = extract_links_from_feed(xml);
        assert_eq!(links, vec!["https://x.com/cdata".to_string()]);
    }

    #[test]
    fn test_robots_disallow() {
        let robots = "User-agent: *\nDisallow: /private\nAllow: /private/public\n";
        let rules = RobotsRules::parse(robots);
        assert!(!rules.is_allowed("/private/secret"));
        assert!(rules.is_allowed("/private/public/page")); // longer allow wins
        assert!(rules.is_allowed("/news/story"));
    }

    #[test]
    fn test_robots_only_applies_to_star_agent() {
        let robots = "User-agent: GoogleBot\nDisallow: /\n\nUser-agent: *\nDisallow: /admin\n";
        let rules = RobotsRules::parse(robots);
        assert!(rules.is_allowed("/news"));       // the Disallow:/ for GoogleBot must not apply
        assert!(!rules.is_allowed("/admin/x"));
    }

    #[test]
    fn test_robots_empty_allows_all() {
        let rules = RobotsRules::parse("");
        assert!(rules.is_allowed("/anything"));
    }

    #[test]
    fn test_throttle_enforces_interval() {
        let host = "throttle-test.example";
        throttle_host(host, Duration::from_millis(0)); // prime
        let start = Instant::now();
        throttle_host(host, Duration::from_millis(120));
        throttle_host(host, Duration::from_millis(120));
        // two back-to-back calls with a 120ms min interval should take >= ~120ms total
        assert!(start.elapsed() >= Duration::from_millis(100));
    }
}
