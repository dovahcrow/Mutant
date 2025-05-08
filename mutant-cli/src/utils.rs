use std::time::Duration;
use humantime::format_duration;
use indicatif::MultiProgress;

/// Format a duration into a human-readable string using the humantime crate
///
/// Examples:
/// - 1m 30s
/// - 2s 500ms
/// - 45ms
pub fn format_elapsed_time(elapsed: Duration) -> String {
    format!("{}", format_duration(elapsed))
}

/// Ensure all progress bars in a MultiProgress are properly cleared
///
/// This function should be called before displaying final messages to ensure
/// that all progress bars are cleared from the terminal.
pub fn ensure_progress_cleared(multi_progress: &MultiProgress) {
    // Force a refresh of the progress bars
    multi_progress.clear().unwrap_or_default();
}
