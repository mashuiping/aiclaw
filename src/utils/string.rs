//! String utilities

use regex::Regex;

/// Prefix of `s` with at most `max_chars` Unicode scalar values (always on UTF-8 boundaries).
pub fn utf8_prefix_chars(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        None => s,
        Some((idx, _)) => &s[..idx],
    }
}

/// Truncate string to at most `max_chars` characters, appending `...` when shortened.
pub fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    if max_chars < 3 {
        return utf8_prefix_chars(s, max_chars).to_string();
    }
    format!("{}...", utf8_prefix_chars(s, max_chars - 3))
}

/// Extract mentions from text
pub fn extract_mentions(text: &str) -> Vec<String> {
    let re = Regex::new(r"@(\S+)").unwrap();
    re.captures_iter(text)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

/// Clean markdown for plain text output
pub fn clean_markdown(s: &str) -> String {
    let s = Regex::new(r"```[\s\S]*?```").unwrap().replace_all(s, "[code]");
    let s = Regex::new(r"`([^`]+)`").unwrap().replace_all(&s, "$1");
    let s = Regex::new(r"\*\*([^*]+)\*\*").unwrap().replace_all(&s, "$1");
    let s = Regex::new(r"\*([^*]+)\*").unwrap().replace_all(&s, "$1");
    let s = Regex::new(r"__([^_]+)__").unwrap().replace_all(&s, "$1");
    let s = Regex::new(r"\[([^\]]+)\]\([^)]+\)").unwrap().replace_all(&s, "$1");
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 8), "hello...");
        assert_eq!(truncate("hi", 2), "hi");
    }

    #[test]
    fn utf8_prefix_does_not_panic_on_multibyte() {
        let s = "为什么 pending";
        assert_eq!(utf8_prefix_chars(s, 3), "为什么");
        assert_eq!(utf8_prefix_chars(s, 100), s);
    }

    #[test]
    fn test_extract_mentions() {
        assert_eq!(extract_mentions("@alice hello @bob"), vec!["alice", "bob"]);
    }
}
