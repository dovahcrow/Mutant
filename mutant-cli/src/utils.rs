use std::time::Duration;
use indicatif::MultiProgress;
use pretty_duration::{PrettyDurationOptions, PrettyDurationOutputFormat};

/// Format a duration into a human-readable string using the humantime crate
/// but hide durations under 1 second
///
/// Examples:
/// - 1m 30s
/// - 2s
/// - <1s (for durations under 1 second)
pub fn format_elapsed_time(elapsed: Duration) -> String {
    let elapsed = pretty_duration::pretty_duration(&elapsed, Some(PrettyDurationOptions {
        output_format: Some(PrettyDurationOutputFormat::Compact),
        singular_labels: None,
        plural_labels: None,
    }));
    
    format!("{}", elapsed)
}

/// Ensure all progress bars in a MultiProgress are properly cleared
///
/// This function should be called before displaying final messages to ensure
/// that all progress bars are cleared from the terminal.
pub fn ensure_progress_cleared(multi_progress: &MultiProgress) {
    // Force a refresh of the progress bars
    multi_progress.clear().unwrap_or_default();
}
