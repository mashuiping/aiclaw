//! Audit logger for tracking all operations

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use tokio::sync::mpsc;
use tracing::{info, warn};

/// Audit event types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditEventType {
    CommandExecution,
    ConfirmationRequested,
    ConfirmationReceived,
    BlockedCommand,
    IntentClassification,
    SkillExecution,
    MCPInvocation,
    Error,
}

impl fmt::Display for AuditEventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuditEventType::CommandExecution => write!(f, "CommandExecution"),
            AuditEventType::ConfirmationRequested => write!(f, "ConfirmationRequested"),
            AuditEventType::ConfirmationReceived => write!(f, "ConfirmationReceived"),
            AuditEventType::BlockedCommand => write!(f, "BlockedCommand"),
            AuditEventType::IntentClassification => write!(f, "IntentClassification"),
            AuditEventType::SkillExecution => write!(f, "SkillExecution"),
            AuditEventType::MCPInvocation => write!(f, "MCPInvocation"),
            AuditEventType::Error => write!(f, "Error"),
        }
    }
}

/// Audit event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub timestamp: DateTime<Utc>,
    pub event_type: AuditEventType,
    pub user_id: String,
    pub channel: String,
    pub session_id: String,
    pub command: Option<String>,
    pub intent: Option<String>,
    pub skill: Option<String>,
    pub success: bool,
    pub risk_level: Option<String>,
    pub error_message: Option<String>,
    pub metadata: HashMap<String, String>,
}

impl AuditEvent {
    pub fn new(event_type: AuditEventType) -> Self {
        Self {
            timestamp: Utc::now(),
            event_type,
            user_id: String::new(),
            channel: String::new(),
            session_id: String::new(),
            command: None,
            intent: None,
            skill: None,
            success: true,
            risk_level: None,
            error_message: None,
            metadata: HashMap::new(),
        }
    }

    pub fn with_user(mut self, user_id: &str) -> Self {
        self.user_id = user_id.to_string();
        self
    }

    pub fn with_channel(mut self, channel: &str) -> Self {
        self.channel = channel.to_string();
        self
    }

    pub fn with_session(mut self, session_id: &str) -> Self {
        self.session_id = session_id.to_string();
        self
    }

    pub fn with_command(mut self, command: &str) -> Self {
        self.command = Some(command.to_string());
        self
    }

    pub fn with_intent(mut self, intent: &str) -> Self {
        self.intent = Some(intent.to_string());
        self
    }

    pub fn with_skill(mut self, skill: &str) -> Self {
        self.skill = Some(skill.to_string());
        self
    }

    pub fn with_success(mut self, success: bool) -> Self {
        self.success = success;
        self
    }

    pub fn with_risk_level(mut self, level: &str) -> Self {
        self.risk_level = Some(level.to_string());
        self
    }

    pub fn with_error(mut self, error: &str) -> Self {
        self.error_message = Some(error.to_string());
        self.success = false;
        self
    }

    pub fn with_metadata(mut self, key: &str, value: &str) -> Self {
        self.metadata.insert(key.to_string(), value.to_string());
        self
    }
}

/// Audit logger for storing and forwarding audit events
pub struct AuditLogger {
    sender: mpsc::Sender<AuditEvent>,
}

impl AuditLogger {
    pub fn new() -> (Self, mpsc::Receiver<AuditEvent>) {
        let (tx, rx) = mpsc::channel(1000);
        (Self { sender: tx }, rx)
    }

    /// Log an audit event
    pub fn log(&self, event: AuditEvent) {
        let _event_str = serde_json::to_string(&event).unwrap_or_default();
        let event_type = event.event_type.clone();
        let user_id = event.user_id.clone();
        let command = event.command.clone();
        match self.sender.try_send(event) {
            Ok(_) => {
                info!("AUDIT: {} - {} - {}", event_type, user_id, command.as_deref().unwrap_or("N/A"));
            }
            Err(e) => {
                warn!("Failed to send audit event: {}", e);
            }
        }
    }

    /// Log command execution
    pub fn log_command(
        &self,
        user_id: &str,
        channel: &str,
        session_id: &str,
        command: &str,
        success: bool,
        risk_level: Option<&str>,
    ) {
        let event = AuditEvent::new(AuditEventType::CommandExecution)
            .with_user(user_id)
            .with_channel(channel)
            .with_session(session_id)
            .with_command(command)
            .with_success(success)
            .with_risk_level(risk_level.unwrap_or("N/A"));

        self.log(event);
    }

    /// Log blocked command
    pub fn log_blocked(
        &self,
        user_id: &str,
        channel: &str,
        session_id: &str,
        command: &str,
        reason: &str,
    ) {
        let event = AuditEvent::new(AuditEventType::BlockedCommand)
            .with_user(user_id)
            .with_channel(channel)
            .with_session(session_id)
            .with_command(command)
            .with_success(false)
            .with_error(reason);

        self.log(event);
    }

    /// Log confirmation request
    pub fn log_confirmation_request(
        &self,
        user_id: &str,
        channel: &str,
        session_id: &str,
        command: &str,
    ) {
        let event = AuditEvent::new(AuditEventType::ConfirmationRequested)
            .with_user(user_id)
            .with_channel(channel)
            .with_session(session_id)
            .with_command(command);

        self.log(event);
    }

    /// Log skill execution
    pub fn log_skill_execution(
        &self,
        user_id: &str,
        channel: &str,
        session_id: &str,
        skill: &str,
        success: bool,
    ) {
        let event = AuditEvent::new(AuditEventType::SkillExecution)
            .with_user(user_id)
            .with_channel(channel)
            .with_session(session_id)
            .with_skill(skill)
            .with_success(success);

        self.log(event);
    }

    /// Log intent classification
    pub fn log_intent_classification(
        &self,
        user_id: &str,
        channel: &str,
        session_id: &str,
        intent: &str,
        confidence: f32,
    ) {
        let event = AuditEvent::new(AuditEventType::IntentClassification)
            .with_user(user_id)
            .with_channel(channel)
            .with_session(session_id)
            .with_intent(intent)
            .with_metadata("confidence", &confidence.to_string());

        self.log(event);
    }
}

impl Default for AuditLogger {
    fn default() -> Self {
        let (tx, _rx) = mpsc::channel(1000);
        Self { sender: tx }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_event_builder() {
        let event = AuditEvent::new(AuditEventType::CommandExecution)
            .with_user("user123")
            .with_channel("feishu")
            .with_session("session456")
            .with_command("kubectl get pods")
            .with_success(true)
            .with_risk_level("Low");

        assert_eq!(event.user_id, "user123");
        assert_eq!(event.command, Some("kubectl get pods".to_string()));
        assert!(event.success);
    }
}
