//! Text helpers shared by Copilot and translation.

/// Text of one page. `number` is 1-based.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageText {
    /// Page number, starting at 1.
    pub number: usize,
    /// The page's text in reading order.
    pub text: String,
}

/// The document text prepared for the model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    /// Text with `[[page N]]` markers.
    pub text: String,
    /// Pages whose text is included (in whole or in part).
    pub pages_included: usize,
    /// Pages in the document that have text.
    pub pages_with_text: usize,
    /// Some text did not fit the budget and was left out.
    pub truncated: bool,
}

/// Lower-case, collapse whitespace, join words hyphenated across lines, expand ligatures and
/// straighten quotes so a quote from the model can be compared with extracted page text.
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = true;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        // "hyphen-\nated" → "hyphenated"
        if c == '-' && chars.peek().is_some_and(|n| *n == '\n' || *n == '\r') {
            while chars.peek().is_some_and(|n| n.is_whitespace()) {
                chars.next();
            }
            continue;
        }
        let mapped: &str = match c {
            '\u{fb00}' => "ff",
            '\u{fb01}' => "fi",
            '\u{fb02}' => "fl",
            '\u{fb03}' => "ffi",
            '\u{fb04}' => "ffl",
            '’' | '‘' | '‛' | '`' => "'",
            '“' | '”' | '„' => "\"",
            '–' | '—' | '‐' | '‑' => "-",
            '\u{a0}' | '\u{2009}' | '\u{202f}' => " ",
            _ => "",
        };
        if !mapped.is_empty() {
            if mapped == " " {
                if !last_space {
                    out.push(' ');
                    last_space = true;
                }
            } else {
                out.push_str(mapped);
                last_space = false;
            }
            continue;
        }
        if c.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else if c == '\u{ad}' {
            // soft hyphen
        } else {
            for l in c.to_lowercase() {
                out.push(l);
            }
            last_space = false;
        }
    }
    out.trim_end().to_string()
}

/// Cut `s` to at most `max` characters on a char boundary.
pub fn clip_chars(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// Assemble the document text with page markers, within `max_chars` characters.
pub fn build_document(pages: &[PageText], max_chars: usize) -> Document {
    let mut text = String::new();
    let mut used = 0usize;
    let mut included = 0usize;
    let mut truncated = false;
    let with_text: Vec<&PageText> = pages.iter().filter(|p| !p.text.trim().is_empty()).collect();
    for p in &with_text {
        let header = format!("[[page {}]]\n", p.number);
        let body = p.text.trim();
        let need = header.chars().count() + body.chars().count() + 2;
        if used + need > max_chars {
            let room = max_chars.saturating_sub(used + header.chars().count() + 2);
            if room > 400 && included == 0 || (room > 2000 && !truncated) {
                text.push_str(&header);
                text.push_str(clip_chars(body, room));
                text.push_str("\n\n");
                included += 1;
            }
            truncated = true;
            break;
        }
        text.push_str(&header);
        text.push_str(body);
        text.push_str("\n\n");
        used += need;
        included += 1;
    }
    Document {
        text,
        pages_included: included,
        pages_with_text: with_text.len(),
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_makes_model_quotes_comparable() {
        let page = "The Cus-\ntomer shall   pay the\nﬁrst invoice – within “30” days.";
        let quote = "customer shall pay the first invoice - within \"30\" days";
        assert!(normalize(page).contains(&normalize(quote)));
        assert_eq!(normalize("  A\u{a0}B\t\nC "), "a b c");
        assert_eq!(normalize("it’s"), "it's");
    }

    #[test]
    fn clip_is_char_safe() {
        assert_eq!(clip_chars("héllo wörld", 5), "héllo");
        assert_eq!(clip_chars("abc", 10), "abc");
    }

    #[test]
    fn document_building_marks_pages_and_respects_the_budget() {
        let pages: Vec<PageText> = (1..=5)
            .map(|n| PageText {
                number: n,
                text: if n == 2 {
                    "  ".into()
                } else {
                    format!("page {n} {}", "x".repeat(100))
                },
            })
            .collect();
        let d = build_document(&pages, 100_000);
        assert!(!d.truncated);
        assert_eq!(d.pages_with_text, 4);
        assert_eq!(d.pages_included, 4);
        assert!(d.text.contains("[[page 1]]") && d.text.contains("[[page 5]]"));
        assert!(!d.text.contains("[[page 2]]"));
        let small = build_document(&pages, 300);
        assert!(small.truncated);
        assert!(small.pages_included < 4);
        assert!(small.text.chars().count() <= 300);
    }
}
