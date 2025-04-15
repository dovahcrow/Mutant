use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::sync::Arc;
use std::time::Duration as StdDuration;

const DEFAULT_TEMPLATE: &str = "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] [{bytes}/{total_bytes} - {bytes_per_sec}], (ETA: {eta}) {msg}";
const DEFAULT_PROGRESS_CHARS: &str = "#>-";
const DEFAULT_STEP_TEMPLATE: &str =
    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] [{pos}/{len}] (ETA: {eta}) {msg}";

pub fn get_default_bytes_style() -> ProgressStyle {
    ProgressStyle::default_bar()
        .template(DEFAULT_TEMPLATE)
        .expect("Invalid byte progress template")
        .progress_chars(DEFAULT_PROGRESS_CHARS)
}

pub fn get_default_steps_style() -> ProgressStyle {
    ProgressStyle::default_bar()
        .template(DEFAULT_STEP_TEMPLATE)
        .expect("Invalid step progress template")
        .progress_chars(DEFAULT_PROGRESS_CHARS)
}

/// A wrapper around indicatif::ProgressBar to encapsulate styling and steady tick logic.
#[derive(Clone)]
pub struct StyledProgressBar {
    pb: Arc<ProgressBar>,
}

impl StyledProgressBar {
    /// Creates a new styled progress bar with default byte-based styling and adds it to the MultiProgress.
    pub fn new(multi_progress: &MultiProgress) -> Self {
        let pb = Arc::new(multi_progress.add(ProgressBar::new(0)));
        pb.set_style(get_default_bytes_style());
        StyledProgressBar { pb }
    }

    /// Creates a new styled progress bar for step-based progress.
    pub fn new_for_steps(multi_progress: &MultiProgress) -> Self {
        let pb = Arc::new(multi_progress.add(ProgressBar::new(0)));
        pb.set_style(get_default_steps_style());
        StyledProgressBar { pb }
    }

    /// Enables steady tick for the progress bar if its length is greater than 0.
    fn enable_steady_tick_if_needed(&self) {
        if self.pb.length().is_some() && self.pb.length() != Some(0) {
            self.pb.enable_steady_tick(StdDuration::from_millis(100));
        }
    }

    /// Sets the length of the progress bar and enables steady tick if needed.
    pub fn set_length(&self, len: u64) {
        self.pb.set_length(len);
        self.enable_steady_tick_if_needed();
    }

    /// Sets the position of the progress bar.
    pub fn set_position(&self, pos: u64) {
        self.pb.set_position(pos);
    }

    /// Returns the current position of the progress bar.
    pub fn position(&self) -> u64 {
        self.pb.position()
    }

    /// Returns the current length of the progress bar.
    pub fn length(&self) -> Option<u64> {
        self.pb.length()
    }

    /// Sets the message of the progress bar.
    #[allow(dead_code)] // Allow unused for now, template doesn't use {msg}
    pub fn set_message(&self, msg: String) {
        self.pb.set_message(msg);
    }

    /// Finishes the progress bar with a message.
    pub fn finish_with_message(&self, msg: impl Into<std::borrow::Cow<'static, str>>) {
        if !self.pb.is_finished() {
            self.pb.finish_with_message(msg);
        }
    }

    /// Finishes the progress bar and clears it.
    pub fn finish_and_clear(&self) {
        if !self.pb.is_finished() {
            self.pb.finish_and_clear();
        }
    }

    /// Abandons the progress bar with a message.
    pub fn abandon_with_message(&self, msg: String) {
        if !self.pb.is_finished() {
            self.pb.abandon_with_message(msg);
        }
    }

    /// Checks if the progress bar is finished.
    pub fn is_finished(&self) -> bool {
        self.pb.is_finished()
    }

    /// Sets the style of the underlying progress bar.
    pub fn set_style(&self, style: ProgressStyle) {
        self.pb.set_style(style);
    }
}
