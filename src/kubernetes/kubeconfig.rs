//! Kubeconfig security utilities

use std::path::Path;
use tracing::{debug, warn};

/// Sensitive path suffixes that should not be accessed
const SENSITIVE_SUFFIXES: &[&str] = &[
    ".kubeconfig",
    ".docker/config.json",
    ".aws/credentials",
    ".gcloud/credentials.json",
    ".azure/credentials",
];

/// Sensitive path components
const SENSITIVE_COMPONENTS: &[&str] = &[
    ".ssh",
    ".aws",
    ".gnupg",
    ".kube",
    ".docker",
    ".azure",
    ".secrets",
    ".vault",
];

/// Check if a path is sensitive and should be protected
pub fn is_sensitive_path(path: &Path) -> bool {
    let path_str = path.to_string_lossy();

    for suffix in SENSITIVE_SUFFIXES {
        if path_str.ends_with(suffix) {
            debug!("Path {} is sensitive (suffix match)", path_str);
            return true;
        }
    }

    for component in SENSITIVE_COMPONENTS {
        if path_str.contains(component) {
            debug!("Path {} is sensitive (component match)", path_str);
            return true;
        }
    }

    false
}

/// Validate kubeconfig path for security
pub fn validate_kubeconfig_path(path: &Path, allowed_paths: &[impl AsRef<Path>]) -> anyhow::Result<()> {
    if !path.exists() {
        anyhow::bail!("Kubeconfig path does not exist: {:?}", path);
    }

    if !path.is_file() {
        anyhow::bail!("Kubeconfig path is not a file: {:?}", path);
    }

    let canonical = path.canonicalize()
        .map_err(|e| anyhow::anyhow!("Failed to canonicalize path: {}", e))?;

    for allowed in allowed_paths {
        let allowed_canonical = allowed.as_ref().canonicalize()
            .map_err(|e| anyhow::anyhow!("Failed to canonicalize allowed path: {}", e))?;

        if canonical == allowed_canonical {
            return Ok(());
        }
    }

    if allowed_paths.is_empty() {
        if is_sensitive_path(path) {
            warn!("Accessing potentially sensitive kubeconfig at {:?}", path);
        }
        return Ok(());
    }

    anyhow::bail!("Kubeconfig path {:?} is not in the allowed list", path)
}

/// Check if kubeconfig contains embedded credentials
pub fn check_embedded_credentials(content: &str) -> Vec<String> {
    let mut issues = Vec::new();

    let danger_patterns = [
        ("password:", "embedded password"),
        ("token:", "embedded token"),
        ("client-secret:", "embedded client secret"),
        ("aws_access_key", "AWS credentials"),
    ];

    for (pattern, description) in &danger_patterns {
        if content.contains(pattern) {
            issues.push(description.to_string());
        }
    }

    issues
}

/// Sanitize kubeconfig content for logging
pub fn sanitize_for_logging(content: &str) -> String {
    let sensitive = [
        "password:",
        "token:",
        "client-secret:",
        "aws_access_key",
        "aws_secret_key",
    ];

    let mut sanitized = content.to_string();
    for pattern in sensitive {
        sanitized = sanitized.replace(pattern, "[REDACTED]");
    }

    sanitized
}
