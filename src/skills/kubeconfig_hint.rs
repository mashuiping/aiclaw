//! Heuristic extraction of kubeconfig file paths from user text.

use std::path::{Path, PathBuf};

use regex::Regex;

/// Expand leading `~` to the home directory when present.
pub fn expand_path(raw: &str) -> PathBuf {
    let raw = raw.trim().trim_matches(|c| c == '"' || c == '\'');
    if let Some(rest) = raw.strip_prefix("~/") {
        return dirs::home_dir()
            .map(|h| h.join(rest))
            .unwrap_or_else(|| PathBuf::from(raw));
    }
    if raw == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(raw));
    }
    PathBuf::from(raw)
}

/// Best-effort: find a kubeconfig path the user mentioned (session override).
pub fn extract_from_user_text(text: &str) -> Option<PathBuf> {
    static RE_ASSIGN: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re_assign = RE_ASSIGN.get_or_init(|| {
        Regex::new(r"(?i)AICLAW_KUBECONFIG\s*=\s*(\S+)").expect("regex")
    });
    if let Some(c) = re_assign.captures(text) {
        let p = expand_path(c.get(1)?.as_str());
        if looks_like_kubeconfig_path(&p) {
            return Some(p);
        }
    }

    static RE_KUBECONFIG_EQ: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re_keq = RE_KUBECONFIG_EQ.get_or_init(|| {
        Regex::new(r"(?i)kubeconfig\s*[:=]\s*(\S+)").expect("regex")
    });
    if let Some(c) = re_keq.captures(text) {
        let p = expand_path(c.get(1)?.as_str());
        if looks_like_kubeconfig_path(&p) {
            return Some(p);
        }
    }

    static RE_ABS: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re_abs = RE_ABS.get_or_init(|| Regex::new(r"(?m)(/[\w./-]{4,})").expect("regex"));
    for m in re_abs.find_iter(text) {
        let p = PathBuf::from(m.as_str());
        if p.is_file() && looks_like_kubeconfig_path(&p) {
            return Some(p);
        }
    }

    None
}

/// Resolve `AICLAW_KUBECONFIG` at process start (absolute path preferred).
pub fn kubeconfig_from_aiclaw_env() -> Option<PathBuf> {
    let raw = std::env::var_os("AICLAW_KUBECONFIG")?;
    let s = raw.to_string_lossy();
    if s.trim().is_empty() {
        return None;
    }
    let mut p = expand_path(s.trim());
    if p.is_relative() {
        if let Ok(cwd) = std::env::current_dir() {
            p = cwd.join(p);
        }
    }
    Some(p)
}

fn looks_like_kubeconfig_path(p: &Path) -> bool {
    let s = p.to_string_lossy();
    if s.contains("..") {
        return false;
    }
    let lower = s.to_lowercase();
    lower.contains("kube") || lower.ends_with(".yaml") || lower.ends_with(".yml")
}
