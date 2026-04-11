//! Command whitelist validator

use std::collections::HashSet;
use tracing::{debug, warn};

/// Command validation result
#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub allowed: bool,
    pub reason: Option<String>,
    pub requires_confirmation: bool,
    pub risk_level: RiskLevel,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// Command whitelist validator
pub struct CommandValidator {
    /// When set, reject every shell command (policy kill-switch).
    deny_all: bool,
    /// Allowed command patterns (e.g., "kubectl get", "kubectl logs")
    allowed_patterns: Vec<CommandPattern>,
    /// Commands that require explicit confirmation
    confirmation_required: Vec<String>,
    /// Sensitive operations
    #[allow(dead_code)]
    sensitive_keywords: HashSet<String>,
    /// Dangerous commands that are always blocked
    blocked_commands: HashSet<String>,
}

#[derive(Debug, Clone)]
struct CommandPattern {
    /// The base command (e.g., "kubectl")
    base: String,
    /// Allowed subcommands (e.g., ["get", "logs", "describe"])
    allowed_subcommands: Vec<String>,
    /// Allowed resource types for this command
    #[allow(dead_code)]
    allowed_resources: Vec<String>,
}

impl CommandValidator {
    pub fn new() -> Self {
        Self {
            deny_all: false,
            allowed_patterns: Vec::new(),
            confirmation_required: Vec::new(),
            sensitive_keywords: HashSet::new(),
            blocked_commands: HashSet::new(),
        }
    }

    /// Reject all commands (used for `skills.exec.security = "deny"`).
    pub fn deny_all() -> Self {
        Self {
            deny_all: true,
            allowed_patterns: Vec::new(),
            confirmation_required: Vec::new(),
            sensitive_keywords: HashSet::new(),
            blocked_commands: HashSet::new(),
        }
    }

    /// Add an allowed kubectl pattern
    #[allow(dead_code)]
    pub fn add_kubectl_pattern(
        mut self,
        subcommands: Vec<String>,
        allowed_resources: Vec<String>,
    ) -> Self {
        self.allowed_patterns.push(CommandPattern {
            base: "kubectl".to_string(),
            allowed_subcommands: subcommands,
            allowed_resources,
        });
        self
    }

    /// Allow read-only `helm` subcommands (HAMi / release discovery).
    pub fn add_helm_read_pattern(mut self) -> Self {
        self.allowed_patterns.push(CommandPattern {
            base: "helm".to_string(),
            allowed_subcommands: vec![
                "list".to_string(),
                "get".to_string(),
                "status".to_string(),
                "version".to_string(),
                "show".to_string(),
                "history".to_string(),
            ],
            allowed_resources: vec![],
        });
        self
    }

    /// Development-only: no program allowlist, still blocks obvious shell hazards.
    pub fn permissive_shell() -> Self {
        Self::new()
            .add_blocked_commands(vec![
                "rm".to_string(),
                "dd".to_string(),
                "mkfs".to_string(),
                ">:".to_string(),
                "|".to_string(),
                "&".to_string(),
                ";".to_string(),
                "eval".to_string(),
                "ssh".to_string(),
                "curl".to_string(),
                "wget".to_string(),
            ])
    }

    /// Add a command that requires confirmation
    pub fn add_confirmation_required(mut self, pattern: String) -> Self {
        self.confirmation_required.push(pattern);
        self
    }

    /// Add dangerous commands that are always blocked
    pub fn add_blocked_commands(mut self, commands: Vec<String>) -> Self {
        for cmd in commands {
            self.blocked_commands.insert(cmd.to_lowercase());
        }
        self
    }

    /// Validate a command string
    pub fn validate(&self, command: &str) -> ValidationResult {
        if self.deny_all {
            return ValidationResult {
                allowed: false,
                reason: Some("Command execution denied by policy (deny-all)".to_string()),
                requires_confirmation: false,
                risk_level: RiskLevel::Critical,
            };
        }

        let parts: Vec<&str> = command.split_whitespace().collect();

        if parts.is_empty() {
            return ValidationResult {
                allowed: false,
                reason: Some("Empty command".to_string()),
                requires_confirmation: false,
                risk_level: RiskLevel::Critical,
            };
        }

        let base = parts[0];

        // Check if command contains any blocked keyword
        let command_lower = command.to_lowercase();
        for blocked in &self.blocked_commands {
            if command_lower.contains(blocked.as_str()) {
                warn!("Blocked command containing '{}': {}", blocked, command);
                return ValidationResult {
                    allowed: false,
                    reason: Some(format!("Command contains blocked keyword: {}", blocked)),
                    requires_confirmation: false,
                    risk_level: RiskLevel::Critical,
                };
            }
        }

        // If no patterns defined, allow all (backwards compatible)
        if self.allowed_patterns.is_empty() {
            return ValidationResult {
                allowed: true,
                reason: None,
                requires_confirmation: false,
                risk_level: RiskLevel::Low,
            };
        }

        // Find matching pattern
        for pattern in &self.allowed_patterns {
            if base == pattern.base && parts.len() >= 2 {
                let subcommand = parts[1];

                // Check if subcommand is allowed
                if !pattern.allowed_subcommands.is_empty()
                    && !pattern.allowed_subcommands.iter().any(|s| s == subcommand)
                {
                    return ValidationResult {
                        allowed: false,
                        reason: Some(format!(
                            "kubectl subcommand '{}' is not allowed. Allowed: {:?}",
                            subcommand, pattern.allowed_subcommands
                        )),
                        requires_confirmation: false,
                        risk_level: RiskLevel::High,
                    };
                }

                // Check if operation requires confirmation
                let requires_confirmation = self.confirmation_required.iter().any(|p| {
                    command.to_lowercase().contains(&p.to_lowercase())
                });

                // Calculate risk level based on subcommand
                let risk_level = match subcommand {
                    "delete" | "stop" | "scale" => RiskLevel::High,
                    "edit" | "patch" | "replace" => RiskLevel::Medium,
                    _ => RiskLevel::Low,
                };

                debug!("Command '{}' validated, risk: {:?}", command, risk_level);

                return ValidationResult {
                    allowed: true,
                    reason: None,
                    requires_confirmation,
                    risk_level,
                };
            }
        }

        // No matching pattern found
        ValidationResult {
            allowed: false,
            reason: Some(format!("Command '{}' is not in the whitelist", base)),
            requires_confirmation: false,
            risk_level: RiskLevel::High,
        }
    }
}

impl Default for CommandValidator {
    fn default() -> Self {
        // Default validator with common safe kubectl commands
        Self::new()
            .add_kubectl_pattern(
                vec![
                    "get".to_string(),
                    "describe".to_string(),
                    "logs".to_string(),
                    "top".to_string(),
                    "events".to_string(),
                    "api-resources".to_string(),
                    "cluster-info".to_string(),
                    "namespace".to_string(),
                    "config".to_string(),
                    "explain".to_string(),
                    "delete".to_string(),
                ],
                vec![
                    "pods".to_string(),
                    "pod".to_string(),
                    "services".to_string(),
                    "service".to_string(),
                    "deployments".to_string(),
                    "deployment".to_string(),
                    "replicasets".to_string(),
                    "replicaset".to_string(),
                    "nodes".to_string(),
                    "node".to_string(),
                    "namespaces".to_string(),
                    "events".to_string(),
                    "configmaps".to_string(),
                    "configmap".to_string(),
                    "secrets".to_string(),
                    "secret".to_string(),
                    "services".to_string(),
                    "endpoints".to_string(),
                    "ingresses".to_string(),
                    "ingress".to_string(),
                    "horizontalpodautoscalers".to_string(),
                    "hpa".to_string(),
                    "persistentvolumes".to_string(),
                    "pv".to_string(),
                    "persistentvolumeclaims".to_string(),
                    "pvc".to_string(),
                    "storageclasses".to_string(),
                    "sc".to_string(),
                ],
            )
            .add_confirmation_required("delete".to_string())
            .add_confirmation_required("scale".to_string())
            .add_confirmation_required("stop".to_string())
            .add_blocked_commands(vec![
                "rm".to_string(),
                "dd".to_string(),
                "mkfs".to_string(),
                ">:".to_string(),
                "|".to_string(),
                "&".to_string(),
                ";".to_string(),
                "eval".to_string(),
                "exec".to_string(),
                "ssh".to_string(),
                "curl".to_string(),
                "wget".to_string(),
            ])
    }
}

/// Security configuration
#[derive(Debug, Clone)]
pub struct SecurityConfig {
    /// Enable command whitelist mode
    pub whitelist_enabled: bool,
    /// Commands requiring confirmation before execution
    pub confirmation_required: Vec<String>,
    /// Blocked keywords
    pub blocked_keywords: Vec<String>,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            whitelist_enabled: true,
            confirmation_required: vec![
                "delete".to_string(),
                "scale".to_string(),
                "stop".to_string(),
                "restart".to_string(),
            ],
            blocked_keywords: vec![
                "rm".to_string(),
                "dd".to_string(),
                "mkfs".to_string(),
                "eval".to_string(),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_validator_allows_safe_commands() {
        let validator = CommandValidator::default();

        // Safe read commands should be allowed
        assert!(validator.validate("kubectl get pods").allowed);
        assert!(validator.validate("kubectl describe pod nginx").allowed);
        assert!(validator.validate("kubectl logs nginx-123").allowed);
        assert!(validator.validate("kubectl top pod").allowed);
    }

    #[test]
    fn test_delete_requires_confirmation() {
        let validator = CommandValidator::default();

        let result = validator.validate("kubectl delete pod nginx");
        assert!(result.allowed);
        assert!(result.requires_confirmation);
        assert_eq!(result.risk_level, RiskLevel::High);
    }

    #[test]
    fn test_blocked_command_rejected() {
        let validator = CommandValidator::default();

        assert!(!validator.validate("kubectl get pods | rm -rf").allowed);
        assert!(!validator.validate("kubectl exec evil").allowed);
    }

    #[test]
    fn test_empty_validator_allows_all() {
        let validator = CommandValidator::new();

        assert!(validator.validate("kubectl delete pod nginx").allowed);
        assert!(validator.validate("kubectl get pods").allowed);
    }
}
