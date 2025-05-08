use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

/// A simple wrapper for MultiProgress
/// This replaces the ProgressWithDisabledStdin from the removed terminal module
pub struct ProgressWrapper {
    multi_progress: MultiProgress,
}

impl ProgressWrapper {
    /// Create a new MultiProgress wrapper
    pub fn new() -> Self {
        Self {
            multi_progress: MultiProgress::new(),
        }
    }

    /// Get a reference to the underlying MultiProgress
    pub fn multi_progress(&self) -> &MultiProgress {
        &self.multi_progress
    }
}

impl std::ops::Deref for ProgressWrapper {
    type Target = MultiProgress;

    fn deref(&self) -> &Self::Target {
        &self.multi_progress
    }
}

pub struct StyledProgressBar {
    progress_bar: ProgressBar,
}

impl StyledProgressBar {
    pub fn new_for_steps(multi_progress: &MultiProgress) -> Self {
        let pb = multi_progress.add(ProgressBar::new(100));
        pb.enable_steady_tick(std::time::Duration::from_millis(100));
        pb.set_style(
            ProgressStyle::default_bar()
                .template(
                    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}",
                )
                .unwrap()
                .progress_chars("#>-"),
        );
        Self { progress_bar: pb }
    }

    pub fn set_message(&self, msg: String) {
        self.progress_bar.set_message(msg);
    }

    pub fn set_length(&self, len: u64) {
        self.progress_bar.set_length(len);
    }

    pub fn set_position(&self, pos: u64) {
        self.progress_bar.set_position(pos);
    }

    pub fn position(&self) -> u64 {
        self.progress_bar.position()
    }

    pub fn length(&self) -> Option<u64> {
        self.progress_bar.length()
    }

    pub fn inc(&self, delta: u64) {
        self.progress_bar.inc(delta);
    }

    pub fn is_finished(&self) -> bool {
        self.progress_bar.is_finished()
    }

    pub fn finish_and_clear(&self) {
        self.progress_bar.finish_and_clear();
    }
}
