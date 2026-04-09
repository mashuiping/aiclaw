//! Multi-tenant isolation implementation

use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::RwLock;
use tracing::{debug, info};

/// Tenant identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TenantId(pub String);

impl TenantId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Tenant configuration
#[derive(Debug, Clone)]
pub struct TenantConfig {
    pub id: TenantId,
    pub name: String,
    /// Allowed channels for this tenant
    pub allowed_channels: Vec<String>,
    /// RBAC enabled for this tenant
    pub rbac_enabled: bool,
    /// Custom cluster access (if empty, all clusters are accessible)
    pub accessible_clusters: Vec<String>,
    /// Rate limiting: max requests per minute
    pub rate_limit: u32,
    /// Data retention days
    pub retention_days: u32,
}

impl TenantConfig {
    pub fn new(id: TenantId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            allowed_channels: vec![],
            rbac_enabled: false,
            accessible_clusters: vec![],
            rate_limit: 60,
            retention_days: 90,
        }
    }

    pub fn with_channels(mut self, channels: Vec<String>) -> Self {
        self.allowed_channels = channels;
        self
    }

    pub fn with_rbac(mut self, enabled: bool) -> Self {
        self.rbac_enabled = enabled;
        self
    }

    pub fn with_clusters(mut self, clusters: Vec<String>) -> Self {
        self.accessible_clusters = clusters;
        self
    }
}

/// Tenant context for request processing
#[derive(Debug, Clone)]
pub struct TenantContext {
    pub tenant_id: TenantId,
    pub user_id: String,
    pub channel: String,
    pub config: Arc<TenantConfig>,
}

/// Tenant manager - manages multiple tenants
pub struct TenantManager {
    tenants: RwLock<HashMap<TenantId, Arc<TenantConfig>>>,
    user_tenants: RwLock<HashMap<String, TenantId>>,
}

impl Default for TenantManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TenantManager {
    pub fn new() -> Self {
        Self {
            tenants: RwLock::new(HashMap::new()),
            user_tenants: RwLock::new(HashMap::new()),
        }
    }

    /// Register a tenant
    pub fn register_tenant(&self, config: TenantConfig) {
        let tenant_id = config.id.clone();
        let tenant = Arc::new(config);
        self.tenants.write().insert(tenant_id.clone(), tenant.clone());
        info!("Registered tenant: {:?}", tenant_id);
    }

    /// Register user to tenant mapping
    pub fn assign_user_to_tenant(&self, user_id: &str, tenant_id: &TenantId) {
        self.user_tenants
            .write()
            .insert(user_id.to_string(), tenant_id.clone());
        debug!("Assigned user {} to tenant {:?}", user_id, tenant_id);
    }

    /// Get tenant for a user
    pub fn get_user_tenant(&self, user_id: &str) -> Option<TenantId> {
        self.user_tenants.read().get(user_id).cloned()
    }

    /// Get tenant config
    pub fn get_tenant(&self, tenant_id: &TenantId) -> Option<Arc<TenantConfig>> {
        self.tenants.read().get(tenant_id).cloned()
    }

    /// Check if user can access channel
    pub fn can_access_channel(&self, user_id: &str, channel: &str) -> bool {
        let tenant_id = match self.get_user_tenant(user_id) {
            Some(id) => id,
            None => return true, // No tenant assigned, allow by default
        };

        let tenant = match self.get_tenant(&tenant_id) {
            Some(t) => t,
            None => return true,
        };

        // If no channels specified, allow all
        if tenant.allowed_channels.is_empty() {
            return true;
        }

        tenant.allowed_channels.contains(&channel.to_string())
    }

    /// Check if user can access cluster
    pub fn can_access_cluster(&self, user_id: &str, cluster: &str) -> bool {
        let tenant_id = match self.get_user_tenant(user_id) {
            Some(id) => id,
            None => return true,
        };

        let tenant = match self.get_tenant(&tenant_id) {
            Some(t) => t,
            None => return true,
        };

        // If no clusters specified, allow all
        if tenant.accessible_clusters.is_empty() {
            return true;
        }

        tenant.accessible_clusters.contains(&cluster.to_string())
    }

    /// Get rate limit for user
    pub fn get_rate_limit(&self, user_id: &str) -> u32 {
        let tenant_id = match self.get_user_tenant(user_id) {
            Some(id) => id,
            None => return 60, // Default rate limit
        };

        let tenant = match self.get_tenant(&tenant_id) {
            Some(t) => t,
            None => return 60,
        };

        tenant.rate_limit
    }

    /// Create tenant context for request
    pub fn create_context(&self, user_id: &str, channel: &str) -> Option<TenantContext> {
        let tenant_id = self.get_user_tenant(user_id)?;
        let config = self.get_tenant(&tenant_id)?;

        Some(TenantContext {
            tenant_id,
            user_id: user_id.to_string(),
            channel: channel.to_string(),
            config,
        })
    }

    /// List all tenants
    pub fn list_tenants(&self) -> Vec<TenantId> {
        self.tenants.read().keys().cloned().collect()
    }

    /// Get tenant count
    pub fn tenant_count(&self) -> usize {
        self.tenants.read().len()
    }
}

/// Rate limiter for tenant
pub struct RateLimiter {
    requests: RwLock<HashMap<String, Vec<std::time::Instant>>>,
    window_secs: u64,
    max_requests: u32,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(60, 60) // 60 requests per minute
    }
}

impl RateLimiter {
    pub fn new(window_secs: u64, max_requests: u32) -> Self {
        Self {
            requests: RwLock::new(HashMap::new()),
            window_secs,
            max_requests,
        }
    }

    /// Check if request is allowed and record it
    pub fn check_and_record(&self, key: &str) -> bool {
        let mut requests = self.requests.write();
        let now = std::time::Instant::now();
        let window = std::time::Duration::from_secs(self.window_secs);

        // Get or create request history
        let history = requests.entry(key.to_string()).or_insert_with(Vec::new);

        // Remove old requests outside the window
        history.retain(|t| now.duration_since(*t) < window);

        // Check rate limit
        if history.len() >= self.max_requests as usize {
            return false;
        }

        // Record this request
        history.push(now);
        true
    }

    /// Get remaining requests for key
    pub fn remaining(&self, key: &str) -> u32 {
        let requests = self.requests.read();
        let now = std::time::Instant::now();
        let window = std::time::Duration::from_secs(self.window_secs);

        if let Some(history) = requests.get(key) {
            let valid_count = history.iter().filter(|t| now.duration_since(**t) < window).count();
            self.max_requests - valid_count as u32
        } else {
            self.max_requests
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tenant_creation() {
        let manager = TenantManager::new();
        let tenant_config = TenantConfig::new(TenantId::new("tenant1"), "Test Tenant")
            .with_channels(vec!["feishu".to_string()])
            .with_clusters(vec!["prod".to_string()]);

        manager.register_tenant(tenant_config);
        manager.assign_user_to_tenant("user1", &TenantId::new("tenant1"));

        assert!(manager.can_access_channel("user1", "feishu"));
        assert!(!manager.can_access_channel("user1", "wecom"));
        assert!(manager.can_access_cluster("user1", "prod"));
        assert!(!manager.can_access_cluster("user1", "dev"));
    }

    #[test]
    fn test_rate_limiter() {
        let limiter = RateLimiter::new(60, 3);

        // First 3 requests should succeed
        assert!(limiter.check_and_record("user1"));
        assert!(limiter.check_and_record("user1"));
        assert!(limiter.check_and_record("user1"));

        // 4th request should fail
        assert!(!limiter.check_and_record("user1"));

        // Different user should succeed
        assert!(limiter.check_and_record("user2"));
    }
}
