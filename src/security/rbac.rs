//! Role-Based Access Control (RBAC) implementation

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, warn};

/// Role types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Administrator - full access
    Admin,
    /// Operator - can execute commands and modify resources
    Operator,
    /// Viewer - read-only access
    Viewer,
}

impl Role {
    /// Check if role has admin privileges
    pub fn is_admin(&self) -> bool {
        matches!(self, Role::Admin)
    }

    /// Get role from string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "admin" | "administrator" => Some(Role::Admin),
            "operator" | "ops" => Some(Role::Operator),
            "viewer" | "read" | "readonly" => Some(Role::Viewer),
            _ => None,
        }
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::Admin => write!(f, "admin"),
            Role::Operator => write!(f, "operator"),
            Role::Viewer => write!(f, "viewer"),
        }
    }
}

/// Permission types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Permission {
    /// Read resources
    Read,
    /// Write/modify resources
    Write,
    /// Execute commands
    Execute,
    /// Administrative operations
    Admin,
    /// Delete resources (sensitive)
    Delete,
    /// Scale resources (sensitive)
    Scale,
}

impl Permission {
    /// Get permission from string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "read" | "get" | "list" => Some(Permission::Read),
            "write" | "create" | "update" | "patch" => Some(Permission::Write),
            "execute" | "exec" | "run" => Some(Permission::Execute),
            "admin" | "administrator" => Some(Permission::Admin),
            "delete" | "remove" => Some(Permission::Delete),
            "scale" | "replica" => Some(Permission::Scale),
            _ => None,
        }
    }
}

/// Intent types that require permission checking
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntentAction {
    Logs,
    Metrics,
    Health,
    Debug,
    Query,
    Scale,
    Deploy,
    Unknown,
}

impl From<&str> for IntentAction {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "logs" => IntentAction::Logs,
            "metrics" => IntentAction::Metrics,
            "health" => IntentAction::Health,
            "debug" => IntentAction::Debug,
            "query" => IntentAction::Query,
            "scale" => IntentAction::Scale,
            "deploy" => IntentAction::Deploy,
            _ => IntentAction::Unknown,
        }
    }
}

/// Permission required for each intent action
fn get_required_permissions(action: IntentAction) -> Vec<Permission> {
    match action {
        IntentAction::Logs => vec![Permission::Read],
        IntentAction::Metrics => vec![Permission::Read],
        IntentAction::Health => vec![Permission::Read],
        IntentAction::Debug => vec![Permission::Read],
        IntentAction::Query => vec![Permission::Read],
        IntentAction::Scale => vec![Permission::Scale, Permission::Write],
        IntentAction::Deploy => vec![Permission::Write, Permission::Execute],
        IntentAction::Unknown => vec![Permission::Read],
    }
}

/// RBAC context for a user request
#[derive(Debug, Clone)]
pub struct RBACContext {
    pub user_id: String,
    pub channel: String,
    pub role: Role,
    pub session_id: Option<String>,
}

/// RBAC validator
pub struct RBACValidator {
    /// Role permissions map
    role_permissions: HashMap<Role, Vec<Permission>>,
    /// User custom roles (user_id -> role)
    user_roles: HashMap<String, Role>,
    /// Sensitive operations requiring confirmation
    sensitive_ops: Vec<IntentAction>,
}

impl Default for RBACValidator {
    fn default() -> Self {
        Self::new()
    }
}

impl RBACValidator {
    pub fn new() -> Self {
        let mut validator = Self {
            role_permissions: HashMap::new(),
            user_roles: HashMap::new(),
            sensitive_ops: vec![IntentAction::Scale, IntentAction::Deploy, IntentAction::Debug],
        };

        // Define default permissions for each role
        validator.role_permissions.insert(
            Role::Admin,
            vec![
                Permission::Read,
                Permission::Write,
                Permission::Execute,
                Permission::Delete,
                Permission::Scale,
                Permission::Admin,
            ],
        );

        validator.role_permissions.insert(
            Role::Operator,
            vec![
                Permission::Read,
                Permission::Write,
                Permission::Execute,
                Permission::Scale,
            ],
        );

        validator.role_permissions.insert(
            Role::Viewer,
            vec![Permission::Read],
        );

        validator
    }

    /// Set role for a user
    pub fn set_user_role(&mut self, user_id: &str, role: Role) {
        self.user_roles.insert(user_id.to_string(), role);
        debug!("Set role {} for user {}", role, user_id);
    }

    /// Get role for a user
    pub fn get_user_role(&self, user_id: &str) -> Role {
        self.user_roles
            .get(user_id)
            .copied()
            .unwrap_or(Role::Viewer)
    }

    /// Check if user has permission for an action
    pub fn check_permission(&self, user_id: &str, action: IntentAction) -> RBACResult {
        let role = self.get_user_role(user_id);
        let required_perms = get_required_permissions(action);

        // Admin can do everything
        if role.is_admin() {
            return RBACResult::Allowed;
        }

        let user_perms = self.role_permissions.get(&role).unwrap_or(&vec![]);

        for required in &required_perms {
            if !user_perms.contains(required) {
                return RBACResult::Denied {
                    reason: format!(
                        "Role {} does not have permission {:?} required for {:?}",
                        role, required, action
                    ),
                };
            }
        }

        // Check if operation requires confirmation
        if self.sensitive_ops.contains(&action) {
            return RBACResult::RequiresConfirmation {
                action: format!("{:?}", action),
                reason: "This is a sensitive operation".to_string(),
            };
        }

        RBACResult::Allowed
    }

    /// Check if operation should be blocked
    pub fn should_block(&self, user_id: &str, action: IntentAction) -> bool {
        matches!(self.check_permission(user_id, action), RBACResult::Denied { .. })
    }

    /// Check if operation requires confirmation
    pub fn requires_confirmation(&self, user_id: &str, action: IntentAction) -> bool {
        matches!(
            self.check_permission(user_id, action),
            RBACResult::RequiresConfirmation { .. }
        )
    }
}

/// RBAC check result
#[derive(Debug, Clone)]
pub enum RBACResult {
    /// Operation allowed
    Allowed,
    /// Operation denied
    Denied { reason: String },
    /// Operation requires user confirmation
    RequiresConfirmation { action: String, reason: String },
}

impl RBACResult {
    pub fn is_allowed(&self) -> bool {
        matches!(self, RBACResult::Allowed)
    }

    pub fn is_denied(&self) -> bool {
        matches!(self, RBACResult::Denied { .. })
    }

    pub fn requires_confirmation(&self) -> bool {
        matches!(self, RBACResult::RequiresConfirmation { .. })
    }
}

/// RBAC middleware for agent orchestrator
pub struct RBACMiddleware {
    validator: Arc<RBACValidator>,
}

impl RBACMiddleware {
    pub fn new(validator: Arc<RBACValidator>) -> Self {
        Self { validator }
    }

    /// Check intent before processing
    pub fn check_intent(&self, user_id: &str, intent_type: &str) -> RBACResult {
        let action = IntentAction::from(intent_type);
        self.validator.check_permission(user_id, action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_role_permissions() {
        let validator = RBACValidator::new();

        // Admin can do everything
        assert!(validator.check_permission("admin", IntentAction::Scale).is_allowed());

        // Viewer can only read
        assert!(validator.check_permission("viewer", IntentAction::Logs).is_allowed());
        assert!(validator.check_permission("viewer", IntentAction::Scale).is_denied());
    }

    #[test]
    fn test_custom_role() {
        let mut validator = RBACValidator::new();
        validator.set_user_role("user1", Role::Operator);

        assert_eq!(validator.get_user_role("user1"), Role::Operator);
        assert!(validator.check_permission("user1", IntentAction::Scale).is_allowed());
    }
}
