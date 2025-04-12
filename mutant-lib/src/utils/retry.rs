use crate::error::Error;
use std::future::Future;
use std::time::Duration;
use tokio_retry::{
    strategy::{jitter, ExponentialBackoff},
    RetryIf,
};

/// Standard retry strategy: Exponential backoff starting at 100ms, 5 attempts, with jitter.
pub fn standard_retry_strategy() -> impl Iterator<Item = Duration> {
    ExponentialBackoff::from_millis(100)
        .max_delay(Duration::from_secs(5))
        .map(jitter)
        .take(5)
}

/// Executes an asynchronous action with a standard retry strategy.
///
/// # Arguments
///
/// * `action_description` - A string describing the action for logging purposes.
/// * `action` - An async closure representing the action to perform. Must be `Fn`.
/// * `retry_if` - A closure that takes a reference to the error and returns true if the action should be retried.
///
/// # Returns
///
/// Returns the result of the action if successful within the retry attempts,
/// otherwise returns the last error encountered.
pub async fn retry_operation<F, Fut, T, E, R>(
    _action_description: &str,
    action: F,
    retry_if: R,
) -> Result<T, E>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Display + From<Error> + std::fmt::Debug, // Add Debug constraint for logging
    R: FnMut(&E) -> bool,
{
    let strategy = standard_retry_strategy();
    let result = RetryIf::spawn(strategy, action, retry_if).await;

    result
}
