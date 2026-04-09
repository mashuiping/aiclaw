//! Retry utilities with exponential backoff

use std::time::Duration;
use tracing::{debug, warn};

/// Retry configuration
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of attempts
    pub max_attempts: u32,
    /// Initial delay between retries
    pub initial_delay: Duration,
    /// Maximum delay between retries
    pub max_delay: Duration,
    /// Multiplier for exponential backoff
    pub multiplier: f64,
    /// Whether to jitter the delay
    pub jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(10),
            multiplier: 2.0,
            jitter: true,
        }
    }
}

impl RetryConfig {
    pub fn new(max_attempts: u32, initial_delay: Duration) -> Self {
        Self {
            max_attempts,
            initial_delay,
            max_delay: Duration::from_secs(30),
            multiplier: 2.0,
            jitter: true,
        }
    }

    pub fn with_max_delay(mut self, max_delay: Duration) -> Self {
        self.max_delay = max_delay;
        self
    }

    pub fn with_multiplier(mut self, multiplier: f64) -> Self {
        self.multiplier = multiplier;
        self
    }

    pub fn no_jitter(mut self) -> Self {
        self.jitter = false;
        self
    }
}

/// Calculate the delay for a given attempt
pub fn calculate_delay(attempt: u32, config: &RetryConfig) -> Duration {
    let delay = config.initial_delay.as_millis() as f64
        * config.multiplier.powf(attempt as f64 - 1.0);

    let delay = delay.min(config.max_delay.as_millis() as f64);

    if config.jitter {
        // Add random jitter between 0% and 25% of the delay
        let jitter_range = delay * 0.25;
        let jitter = (rand_jitter() * jitter_range) as u64;
        Duration::from_millis(delay as u64 + jitter)
    } else {
        Duration::from_millis(delay as u64)
    }
}

/// Simple pseudo-random function for jitter (0.0 to 1.0)
fn rand_jitter() -> f64 {
    use std::time::Instant;
    let now = Instant::now();
    let seed = now.elapsed().as_nanos() % 1000;
    (seed as f64) / 1000.0
}

/// Execute a function with retry
pub async fn with_retry<F, Fut, T, E>(
    mut f: F,
    config: RetryConfig,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let mut last_error = None;

    for attempt in 1..=config.max_attempts {
        match f().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                last_error = Some(e);
                if attempt < config.max_attempts {
                    let delay = calculate_delay(attempt, &config);
                    debug!(
                        "Attempt {} failed, retrying in {:?}...",
                        attempt, delay
                    );
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    Err(last_error.unwrap())
}

/// Execute a function with retry and custom condition
pub async fn with_retry_if<F, Fut, T, E>(
    mut f: F,
    config: RetryConfig,
    should_retry: impl Fn(&E) -> bool,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let mut last_error = None;

    for attempt in 1..=config.max_attempts {
        match f().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                last_error = Some(e);
                if should_retry(&last_error.as_ref().unwrap())
                    && attempt < config.max_attempts
                {
                    let delay = calculate_delay(attempt, &config);
                    debug!(
                        "Attempt {} failed (retryable), retrying in {:?}...",
                        attempt, delay
                    );
                    tokio::time::sleep(delay).await;
                } else if attempt < config.max_attempts {
                    let delay = calculate_delay(attempt, &config);
                    debug!(
                        "Attempt {} failed (condition), retrying in {:?}...",
                        attempt, delay
                    );
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    Err(last_error.unwrap())
}

/// Circuit breaker state
#[derive(Debug, Clone, PartialEq)]
pub enum CircuitState {
    Closed,     // Normal operation
    Open,       // Failing, reject requests
    HalfOpen,   // Testing if service recovered
}

/// Circuit breaker for failing services
#[derive(Debug)]
pub struct CircuitBreaker {
    /// Current state
    state: CircuitState,
    /// Number of failures before opening
    failure_threshold: u32,
    /// Number of successes before closing
    success_threshold: u32,
    /// Current failure count
    failures: u32,
    /// Current success count (in half-open state)
    successes: u32,
    /// When to attempt half-open
    next_attempt: Option<std::time::Instant>,
}

impl CircuitBreaker {
    pub fn new(failure_threshold: u32, success_threshold: u32) -> Self {
        Self {
            state: CircuitState::Closed,
            failure_threshold,
            success_threshold,
            failures: 0,
            successes: 0,
            next_attempt: None,
        }
    }

    /// Check if request can proceed
    pub fn can_proceed(&self) -> bool {
        match self.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                if let Some(next) = self.next_attempt {
                    if std::time::Instant::now() >= next {
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => true,
        }
    }

    /// Record a successful call
    pub fn record_success(&mut self) {
        match self.state {
            CircuitState::Closed => {
                self.failures = 0;
            }
            CircuitState::HalfOpen => {
                self.successes += 1;
                if self.successes >= self.success_threshold {
                    debug!("Circuit breaker closing after {} successes", self.successes);
                    self.state = CircuitState::Closed;
                    self.failures = 0;
                    self.successes = 0;
                    self.next_attempt = None;
                }
            }
            CircuitState::Open => {}
        }
    }

    /// Record a failed call
    pub fn record_failure(&mut self) {
        match self.state {
            CircuitState::Closed => {
                self.failures += 1;
                if self.failures >= self.failure_threshold {
                    debug!(
                        "Circuit breaker opening after {} failures",
                        self.failures
                    );
                    self.state = CircuitState::Open;
                    self.next_attempt =
                        Some(std::time::Instant::now() + Duration::from_secs(30));
                }
            }
            CircuitState::HalfOpen => {
                debug!("Circuit breaker re-opening after failure in half-open state");
                self.state = CircuitState::Open;
                self.next_attempt =
                    Some(std::time::Instant::now() + Duration::from_secs(30));
                self.successes = 0;
            }
            CircuitState::Open => {}
        }
    }

    /// Get current state
    pub fn state(&self) -> &CircuitState {
        &self.state
    }
}

use std::time::Duration;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_config_default() {
        let config = RetryConfig::default();
        assert_eq!(config.max_attempts, 3);
    }

    #[test]
    fn test_circuit_breaker_initial_state() {
        let cb = CircuitBreaker::new(3, 2);
        assert_eq!(cb.state, CircuitState::Closed);
        assert!(cb.can_proceed());
    }

    #[test]
    fn test_circuit_breaker_opens_after_failures() {
        let mut cb = CircuitBreaker::new(3, 2);

        cb.record_failure();
        assert!(cb.can_proceed());
        cb.record_failure();
        assert!(cb.can_proceed());
        cb.record_failure(); // Third failure

        assert_eq!(cb.state, CircuitState::Open);
        assert!(!cb.can_proceed());
    }

    #[test]
    fn test_circuit_breaker_half_open_after_timeout() {
        let mut cb = CircuitBreaker::new(2, 2);

        cb.record_failure();
        cb.record_failure(); // Opens

        assert_eq!(cb.state, CircuitState::Open);

        // Simulate time passing
        cb.next_attempt = Some(std::time::Instant::now() - Duration::from_secs(1));

        assert!(cb.can_proceed());
        assert_eq!(cb.state, CircuitState::HalfOpen);
    }
}
