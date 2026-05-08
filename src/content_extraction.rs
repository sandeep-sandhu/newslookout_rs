// file: content_extraction.rs
// Article content extractor for news HTML pages.
// Uses content-extractor-rl's BaselineExtractor as the primary method,
// falling back to a heuristic CSS-selector implementation when the RL
// extractor fails or returns content below the quality threshold.

use std::collections::HashSet;
use log::{debug, warn};
use scraper::{Html, Selector};
use content_extractor_rl::BaselineExtractor;

// ---------------------------------------------------------------------------
// Stopwords used by BaselineExtractor's scoring heuristic
// ---------------------------------------------------------------------------

fn english_stopwords() -> HashSet<String> {
    [
        "the", "a", "an", "and", "or", "but", "in", "on", "at", "to", "for",
        "of", "with", "by", "from", "is", "are", "was", "were", "be", "been",
        "being", "have", "has", "had", "do", "does", "did", "will", "would",
        "could", "should", "may", "might", "shall", "can", "that", "this",
        "these", "those", "it", "its", "as", "if", "not", "no", "nor", "so",
        "yet", "both", "either", "neither", "each", "few", "more", "most",
        "other", "some", "such", "than", "then", "there", "when", "where",
        "who", "which", "how", "all", "any", "because", "before", "after",
        "while", "about", "into", "through", "during", "above", "below",
        "between", "among", "against", "he", "she", "they", "we", "you",
        "his", "her", "their", "our", "your", "my", "me", "him", "us", "them",
        "what", "said", "also", "just", "like", "up", "out", "over", "new",
        "only", "one", "two", "first", "last", "long", "great", "little",
        "own", "right", "old", "big", "high", "different", "small", "large",
        "next", "early", "young", "important", "public", "private", "real",
        "i", "s", "t", "re", "ve", "ll", "d", "m", "its",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

// ---------------------------------------------------------------------------
// Primary extractor: content-extractor-rl BaselineExtractor
// ---------------------------------------------------------------------------

/// Try to extract article text using the RL-based BaselineExtractor.
/// Returns (text, quality_score) on success, None if extraction failed or
/// produced no usable content.
fn rl_extract(html: &str) -> Option<(String, f32)> {
    let extractor = BaselineExtractor::new(english_stopwords());
    match extractor.extract(html) {
        Ok(result) if !result.text.is_empty() => {
            Some((result.text, result.quality_score))
        }
        Ok(_) => {
            debug!("RL extractor returned empty text");
            None
        }
        Err(e) => {
            debug!("RL extractor error: {}", e);
            None
        }
    }
}

/// Try to extract the article title using the RL extractor's metadata.
fn rl_extract_title(html: &str) -> Option<String> {
    let extractor = BaselineExtractor::new(english_stopwords());
    match extractor.extract(html) {
        Ok(result) => result.title.filter(|t| !t.is_empty()),
        Err(_) => None,
    }
}

// ---------------------------------------------------------------------------
// Fallback extractor: heuristic CSS-selector approach
// ---------------------------------------------------------------------------

/// Candidate content block with quality metrics.
struct ContentBlock {
    text: String,
    word_count: usize,
}

/// Try to extract the main article text from HTML using heuristic selectors.
/// Returns the best candidate block with a quality score, or None.
fn baseline_extract(html: &str) -> Option<(String, f32)> {
    let document = Html::parse_document(html);

    // Ordered list of CSS selectors tried for article body content.
    // More specific / semantic selectors are tried first.
    let article_selectors = [
        "article",
        "[itemprop='articleBody']",
        "[class*='article-body']",
        "[class*='article_body']",
        "[class*='articleBody']",
        "[class*='story-body']",
        "[class*='story_body']",
        "[class*='entry-content']",
        "[class*='post-content']",
        "[class*='content-body']",
        "main",
        "[role='main']",
    ];

    let mut best_block: Option<ContentBlock> = None;

    for selector_str in &article_selectors {
        if let Ok(selector) = Selector::parse(selector_str) {
            for element in document.select(&selector) {
                let mut paragraphs = Vec::new();
                if let Ok(p_sel) = Selector::parse("p") {
                    for p in element.select(&p_sel) {
                        let text: String = p.text().collect();
                        let trimmed = text.trim().to_string();
                        if trimmed.len() > 30 {
                            paragraphs.push(trimmed);
                        }
                    }
                }

                let combined = if paragraphs.is_empty() {
                    element.text().collect::<String>().trim().to_string()
                } else {
                    paragraphs.join("\n\n")
                };

                let word_count = combined.split_whitespace().count();
                let should_replace = match &best_block {
                    None => word_count > 30,
                    Some(prev) => word_count > prev.word_count,
                };

                if should_replace && word_count > 30 {
                    best_block = Some(ContentBlock { text: combined, word_count });
                }
            }
        }
    }

    if let Some(block) = best_block {
        let quality = (block.word_count as f32 / 500.0).min(1.0);
        return Some((block.text, quality));
    }

    // Last-resort fallback: collect all <p> tags from the document.
    if let Ok(p_sel) = Selector::parse("p") {
        let paragraphs: Vec<String> = document
            .select(&p_sel)
            .map(|p| p.text().collect::<String>().trim().to_string())
            .filter(|t| t.len() > 30)
            .collect();

        if !paragraphs.is_empty() {
            let text = paragraphs.join("\n\n");
            let word_count = text.split_whitespace().count();
            // Lower quality multiplier (0.5×) to signal this is a rough fallback.
            let quality = (word_count as f32 / 500.0).min(1.0) * 0.5;
            return Some((text, quality));
        }
    }

    None
}

fn extract_title_from_html(html: &str) -> Option<String> {
    let document = Html::parse_document(html);

    if let Ok(h1_sel) = Selector::parse("h1") {
        if let Some(h1) = document.select(&h1_sel).next() {
            let title: String = h1.text().collect::<String>().trim().to_string();
            if !title.is_empty() {
                return Some(title);
            }
        }
    }

    if let Ok(og_sel) = Selector::parse("meta[property='og:title']") {
        if let Some(meta) = document.select(&og_sel).next() {
            if let Some(content) = meta.value().attr("content") {
                let title = content.trim().to_string();
                if !title.is_empty() {
                    return Some(title);
                }
            }
        }
    }

    if let Ok(title_sel) = Selector::parse("title") {
        if let Some(title_el) = document.select(&title_sel).next() {
            let title: String = title_el.text().collect::<String>().trim().to_string();
            if !title.is_empty() {
                return Some(title);
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Extract article content from HTML.
///
/// Tries the RL-based extractor first; falls back to the heuristic CSS-selector
/// extractor when the RL result is empty or below `min_quality_score`.
///
/// # Arguments
/// * `html`             – Full HTML of the page.
/// * `min_quality_score`– Minimum quality score (0.0–1.0) required to accept
///                        the extracted content.
///
/// # Returns
/// The extracted text on success, or `None` when no content meeting the
/// quality threshold can be found.
pub fn extract_article_content(html: &str, min_quality_score: f32) -> Option<String> {
    // --- Primary: RL-based extractor ---
    if let Some((content, quality)) = rl_extract(html) {
        if quality >= min_quality_score && !content.is_empty() {
            debug!("RL extractor succeeded (quality={:.2})", quality);
            return Some(content);
        }
        debug!("RL extractor quality {:.2} below threshold {:.2}, trying heuristic", quality, min_quality_score);
    }

    // --- Fallback: heuristic CSS-selector extractor ---
    match baseline_extract(html) {
        Some((content, quality_score)) => {
            if quality_score >= min_quality_score && !content.is_empty() {
                debug!("Heuristic extractor succeeded (quality={:.2})", quality_score);
                Some(content)
            } else {
                warn!("Heuristic extraction quality {:.2} below threshold {:.2}", quality_score, min_quality_score);
                None
            }
        }
        None => {
            warn!("Content extraction found no content");
            None
        }
    }
}

/// Extract the article title from HTML.
///
/// Tries the RL extractor's metadata first; falls back to h1 → og:title → title
/// heuristics.
///
/// # Arguments
/// * `html` – Full HTML of the page.
pub fn extract_article_title(html: &str) -> Option<String> {
    rl_extract_title(html).or_else(|| extract_title_from_html(html))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_article_content_empty_html() {
        let result = extract_article_content("<html><body></body></html>", 0.0);
        assert!(result.is_none() || result.unwrap_or_default().is_empty());
    }

    #[test]
    fn test_extract_article_content_with_article_tag() {
        let html = r#"<html><body>
            <article>
                <h1>Test Article Title</h1>
                <p>This is the first paragraph of the article with some meaningful content here.</p>
                <p>This is the second paragraph with more content for the article body text.</p>
                <p>This is the third paragraph of additional content for the test case result.</p>
                <p>Fourth paragraph adds more words so the quality score rises above zero.</p>
                <p>Fifth paragraph ensures we have enough text to pass the minimum threshold.</p>
            </article>
        </body></html>"#;
        let result = extract_article_content(html, 0.0);
        assert!(result.is_some(), "Should extract content from <article> tag");
        let content = result.unwrap();
        assert!(content.contains("first paragraph"));
    }

    #[test]
    fn test_extract_article_content_fallback_paragraphs() {
        let html = r#"<html><body>
            <div class="page">
                <p>A first standalone paragraph with enough words to be counted as content.</p>
                <p>A second standalone paragraph providing additional text for the extractor.</p>
                <p>Third paragraph to ensure we have enough total content for extraction.</p>
            </div>
        </body></html>"#;
        let result = extract_article_content(html, 0.0);
        assert!(result.is_some(), "Should fall back to paragraph extraction");
    }

    #[test]
    fn test_extract_article_title_h1() {
        let html = r#"<html><head><title>Page Title</title></head><body>
            <h1>Article Heading</h1>
        </body></html>"#;
        let title = extract_article_title(html);
        assert!(title.is_some());
        // h1 or title may be returned depending on which extractor wins
        let t = title.unwrap();
        assert!(!t.is_empty());
    }

    #[test]
    fn test_extract_article_title_og_meta() {
        let html = r#"<html><head>
            <meta property="og:title" content="Open Graph Title"/>
            <title>Site Name</title>
        </head><body><p>Content here.</p></body></html>"#;
        let title = extract_article_title(html);
        assert!(title.is_some());
    }

    #[test]
    fn test_extract_article_title_fallback_to_title_tag() {
        let html = r#"<html><head><title>Page Title - Site Name</title></head><body>
            <p>Some content here without an h1.</p>
        </body></html>"#;
        let title = extract_article_title(html);
        assert!(title.is_some());
        assert!(title.unwrap().contains("Page Title"));
    }

    #[test]
    fn test_english_stopwords_not_empty() {
        let sw = english_stopwords();
        assert!(sw.len() > 30);
        assert!(sw.contains("the"));
        assert!(sw.contains("and"));
    }

    #[test]
    fn test_quality_threshold_respected() {
        // A page with almost no text should not pass a high quality threshold.
        let html = r#"<html><body><article><p>Too short.</p></article></body></html>"#;
        let result = extract_article_content(html, 0.9);
        assert!(result.is_none(), "Short content should not pass high quality threshold");
    }
}
