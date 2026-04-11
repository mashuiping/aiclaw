//! Build [`CommandValidator`](crate::security::command_validator::CommandValidator) from `[skills.exec]` config.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::config::{SkillsExecConfig, SkillsExecSecurity};
use crate::security::command_validator::CommandValidator;
use crate::skills::SkillExecutor;

/// Validator used for both LLM-driven shell and declarative `SKILL.toml` tools.
pub fn command_validator_for_skills_exec(cfg: &SkillsExecConfig) -> Arc<CommandValidator> {
    let validator = match cfg.security {
        SkillsExecSecurity::Deny => CommandValidator::deny_all(),
        SkillsExecSecurity::Full => CommandValidator::permissive_shell(),
        SkillsExecSecurity::Allowlist => {
            let base = CommandValidator::default();
            if cfg.allow_helm {
                base.add_helm_read_pattern()
            } else {
                base
            }
        }
    };
    Arc::new(validator)
}

/// Shared [`SkillExecutor`] with timeout and policy from `[skills.exec]`.
pub fn skill_executor_for_config(
    cfg: &SkillsExecConfig,
    kubeconfig: Option<PathBuf>,
) -> Arc<SkillExecutor> {
    let v = command_validator_for_skills_exec(cfg);
    let secs = cfg.timeout_secs.max(1);
    Arc::new(
        SkillExecutor::with_validator_and_kubeconfig(v, kubeconfig)
            .with_timeout(Duration::from_secs(secs)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{SkillsExecConfig, SkillsExecSecurity};

    #[test]
    fn deny_security_blocks_kubectl() {
        let mut cfg = SkillsExecConfig::default();
        cfg.security = SkillsExecSecurity::Deny;
        let v = command_validator_for_skills_exec(&cfg);
        let r = v.validate("kubectl get pods");
        assert!(!r.allowed);
    }

    #[test]
    fn allowlist_security_allows_kubectl_get() {
        let cfg = SkillsExecConfig::default();
        let v = command_validator_for_skills_exec(&cfg);
        let r = v.validate("kubectl get pods");
        assert!(r.allowed);
    }
}
