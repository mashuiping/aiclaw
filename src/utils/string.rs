//! String utilities

use regex::Regex;

/// Truncate string to max length with ellipsis
pub fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else if max_len < 3 {
        s[..max_len].to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
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
    fn test_extract_mentions() {
        assert_eq!(extract_mentions("@alice hello @bob"), vec!["alice", "bob"]);
    }
}
