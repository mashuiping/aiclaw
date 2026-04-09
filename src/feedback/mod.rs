//! Feedback collection module

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info};

/// Feedback type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackType {
    ThumbsUp,
    ThumbsDown,
}

/// User feedback record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackRecord {
    pub id: String,
    pub session_id: String,
    pub user_id: String,
    pub message_index: u32,
    pub feedback_type: FeedbackType,
    pub original_query: String,
    pub response_preview: String,
    pub timestamp: DateTime<Utc>,
    /// Optional user comment
    pub comment: Option<String>,
    /// Optional correct result provided by user
    pub correction: Option<String>,
}

impl FeedbackRecord {
    pub fn new(
        session_id: &str,
        user_id: &str,
        message_index: u32,
        feedback_type: FeedbackType,
        original_query: &str,
        response_preview: &str,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            user_id: user_id.to_string(),
            message_index,
            feedback_type,
            original_query: original_query.to_string(),
            response_preview: response_preview.chars().take(200).collect(),
            timestamp: Utc::now(),
            comment: None,
            correction: None,
        }
    }

    pub fn with_comment(mut self, comment: String) -> Self {
        self.comment = Some(comment);
        self
    }

    pub fn with_correction(mut self, correction: String) -> Self {
        self.correction = Some(correction);
        self
    }
}

/// Skill quality metrics
#[derive(Debug, Clone, Default)]
pub struct SkillMetrics {
    pub total_uses: u32,
    pub successful_uses: u32,
    pub failed_uses: u32,
    pub thumbs_up: u32,
    pub thumbs_down: u32,
    pub average_rating: f32,
}

impl SkillMetrics {
    pub fn success_rate(&self) -> f32 {
        if self.total_uses == 0 {
            return 0.0;
        }
        self.successful_uses as f32 / self.total_uses as f32
    }

    pub fn satisfaction_rate(&self) -> f32 {
        let total = self.thumbs_up + self.thumbs_down;
        if total == 0 {
            return 0.0;
        }
        self.thumbs_up as f32 / total as f32
    }

    pub fn quality_score(&self) -> f32 {
        // Weighted score: 40% success rate, 40% satisfaction, 20% usage
        let usage_score = (self.total_uses.min(100) as f32) / 100.0;
        0.4 * self.success_rate() + 0.4 * self.satisfaction_rate() + 0.2 * usage_score
    }
}

/// Feedback collector - collects and analyzes user feedback
pub struct FeedbackCollector {
    /// Feedback records
    records: DashMap<String, Vec<FeedbackRecord>>,
    /// Skill metrics
    skill_metrics: DashMap<String, SkillMetrics>,
    /// Session message counts
    session_messages: DashMap<String, u32>,
}

impl Default for FeedbackCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl FeedbackCollector {
    pub fn new() -> Self {
        Self {
            records: DashMap::new(),
            skill_metrics: DashMap::new(),
            session_messages: DashMap::new(),
        }
    }

    /// Record user feedback
    pub fn record_feedback(&self, feedback: FeedbackRecord) {
        let session_id = feedback.session_id.clone();
        self.records
            .entry(session_id.clone())
            .or_insert_with(Vec::new)
            .push(feedback.clone());

        debug!(
            "Recorded {:?} feedback for session {}",
            feedback.feedback_type, session_id
        );
    }

    /// Increment session message count
    pub fn increment_message_count(&self, session_id: &str) -> u32 {
        let count = self.session_messages.entry(session_id.to_string()).or_insert(0);
        *count += 1;
        *count
    }

    /// Get current message index for session
    pub fn get_message_index(&self, session_id: &str) -> u32 {
        self.session_messages.get(session_id).copied().unwrap_or(0)
    }

    /// Record skill usage
    pub fn record_skill_usage(
        &self,
        skill_name: &str,
        success: bool,
        feedback: Option<FeedbackType>,
    ) {
        let mut metrics = self
            .skill_metrics
            .entry(skill_name.to_string())
            .or_insert_with(SkillMetrics::default);

        metrics.total_uses += 1;
        if success {
            metrics.successful_uses += 1;
        } else {
            metrics.failed_uses += 1;
        }

        if let Some(fb) = feedback {
            match fb {
                FeedbackType::ThumbsUp => metrics.thumbs_up += 1,
                FeedbackType::ThumbsDown => metrics.thumbs_down += 1,
            }
        }

        // Update average rating
        let total = metrics.thumbs_up + metrics.thumbs_down;
        if total > 0 {
            metrics.average_rating = metrics.thumbs_up as f32 / total as f32;
        }

        info!(
            "Skill {} metrics updated: {} uses, {:.1}% success, {:.1}% satisfaction",
            skill_name,
            metrics.total_uses,
            metrics.success_rate() * 100.0,
            metrics.satisfaction_rate() * 100.0
        );
    }

    /// Get metrics for a skill
    pub fn get_skill_metrics(&self, skill_name: &str) -> Option<SkillMetrics> {
        self.skill_metrics.get(skill_name).map(|m| m.clone())
    }

    /// Get all skills with metrics
    pub fn get_all_skill_metrics(&self) -> Vec<(String, SkillMetrics)> {
        self.skill_metrics
            .iter()
            .map(|m| (m.key().clone(), m.value().clone()))
            .collect()
    }

    /// Get low-quality skills (for review)
    pub fn get_low_quality_skills(&self, threshold: f32) -> Vec<(String, SkillMetrics)> {
        self.skill_metrics
            .iter()
            .filter(|m| m.value().quality_score() < threshold && m.value().total_uses >= 5)
            .map(|m| (m.key().clone(), m.value().clone()))
            .collect()
    }

    /// Get feedback for a session
    pub fn get_session_feedback(&self, session_id: &str) -> Vec<FeedbackRecord> {
        self.records
            .get(session_id)
            .map(|r| r.clone())
            .unwrap_or_default()
    }

    /// Get recent negative feedback (for analysis)
    pub fn get_recent_negative_feedback(&self, limit: usize) -> Vec<FeedbackRecord> {
        let mut all_negative: Vec<FeedbackRecord> = self
            .records
            .iter()
            .flat_map(|r| {
                r.value()
                    .iter()
                    .filter(|f| f.feedback_type == FeedbackType::ThumbsDown)
                    .cloned()
            })
            .collect();

        all_negative.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        all_negative.truncate(limit);
        all_negative
    }

    /// Get feedback statistics
    pub fn get_stats(&self) -> FeedbackStats {
        let total_feedback = self.records.iter().map(|r| r.value().len()).sum();
        let total_thumbs_up: usize = self
            .records
            .iter()
            .map(|r| r.value().iter().filter(|f| f.feedback_type == FeedbackType::ThumbsUp).count())
            .sum();
        let total_thumbs_down = total_feedback - total_thumbs_up;

        let skills_with_metrics = self.skill_metrics.len();
        let low_quality_count = self
            .skill_metrics
            .iter()
            .filter(|m| m.value().quality_score() < 0.5 && m.value().total_uses >= 5)
            .count();

        FeedbackStats {
            total_feedback,
            total_thumbs_up,
            total_thumbs_down,
            satisfaction_rate: if total_feedback > 0 {
                total_thumbs_up as f32 / total_feedback as f32
            } else {
                0.0
            },
            skills_with_metrics,
            low_quality_skills: low_quality_count,
        }
    }
}

/// Feedback statistics
#[derive(Debug, Clone)]
pub struct FeedbackStats {
    pub total_feedback: usize,
    pub total_thumbs_up: usize,
    pub total_thumbs_down: usize,
    pub satisfaction_rate: f32,
    pub skills_with_metrics: usize,
    pub low_quality_skills: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feedback_recording() {
        let collector = FeedbackCollector::new();

        let feedback = FeedbackRecord::new(
            "session1",
            "user1",
            1,
            FeedbackType::ThumbsUp,
            "查看 pod 日志",
            "以下是 pod xxx 的日志...",
        );

        collector.record_feedback(feedback);
        assert_eq!(collector.get_session_feedback("session1").len(), 1);
    }

    #[test]
    fn test_skill_metrics() {
        let collector = FeedbackCollector::new();

        collector.record_skill_usage("kubectl-logs", true, Some(FeedbackType::ThumbsUp));
        collector.record_skill_usage("kubectl-logs", true, Some(FeedbackType::ThumbsUp));
        collector.record_skill_usage("kubectl-logs", false, Some(FeedbackType::ThumbsDown));

        let metrics = collector.get_skill_metrics("kubectl-logs").unwrap();
        assert_eq!(metrics.total_uses, 3);
        assert_eq!(metrics.successful_uses, 2);
        assert_eq!(metrics.failed_uses, 1);
        assert!((metrics.satisfaction_rate() - 0.666).abs() < 0.01);
    }
}
