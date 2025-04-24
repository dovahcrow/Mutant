use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::sync::Arc;
use std::time::Duration as StdDuration;

const DEFAULT_TEMPLATE: &str = "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] [{bytes}/{total_bytes} - {bytes_per_sec}], (ETA: {eta}) {msg}";
const DEFAULT_PROGRESS_CHARS: &str = "#>-";
const DEFAULT_STEP_TEMPLATE: &str =
    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] [{pos}/{len}] (ETA: {eta}) {msg}";
const DEFAULT_SPINNER_TEMPLATE: &str = "{spinner:.green} [{elapsed_precise}] {msg}";

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

pub fn get_default_spinner_style() -> ProgressStyle {
    ProgressStyle::default_spinner()
        .template(DEFAULT_SPINNER_TEMPLATE)
        .expect("Invalid spinner progress template")
}

#[derive(Clone)]
pub struct StyledProgressBar {
    pb: Arc<ProgressBar>,
}

impl StyledProgressBar {
    pub fn new(multi_progress: &MultiProgress) -> Self {
        let pb = Arc::new(multi_progress.add(ProgressBar::new(0)));
        pb.set_style(get_default_bytes_style());
        pb.enable_steady_tick(StdDuration::from_millis(500));
        StyledProgressBar { pb }
    }

    pub fn new_for_steps(multi_progress: &MultiProgress) -> Self {
        let pb = Arc::new(multi_progress.add(ProgressBar::new(0)));
        pb.set_style(get_default_steps_style());
        pb.enable_steady_tick(StdDuration::from_millis(500));
        StyledProgressBar { pb }
    }

    pub fn new_with_style(multi_progress: &MultiProgress, style: ProgressStyle) -> Self {
        let pb = Arc::new(multi_progress.add(ProgressBar::new(0)));
        pb.set_style(style);
        pb.enable_steady_tick(StdDuration::from_millis(500));
        StyledProgressBar { pb }
    }

    pub fn enable_steady_tick(&self, duration: StdDuration) {
        self.pb.enable_steady_tick(duration);
    }

    pub fn set_length(&self, len: u64) {
        self.pb.set_length(len);
        self.enable_steady_tick(StdDuration::from_millis(500));
    }

    pub fn set_position(&self, pos: u64) {
        self.pb.set_position(pos);
    }

    pub fn inc(&self, delta: u64) {
        self.pb.inc(delta);
    }

    pub fn position(&self) -> u64 {
        self.pb.position()
    }

    pub fn length(&self) -> Option<u64> {
        self.pb.length()
    }

    #[allow(dead_code)]
    pub fn set_message(&self, msg: String) {
        self.pb.set_message(msg);
    }

    pub fn finish_with_message(&self, msg: impl Into<std::borrow::Cow<'static, str>>) {
        if !self.pb.is_finished() {
            self.pb.finish_with_message(msg);
        }
    }

    pub fn finish_and_clear(&self) {
        if !self.pb.is_finished() {
            self.pb.finish_and_clear();
        }
    }

    pub fn abandon_with_message(&self, msg: String) {
        if !self.pb.is_finished() {
            self.pb.abandon_with_message(msg);
        }
    }

    pub fn is_finished(&self) -> bool {
        self.pb.is_finished()
    }

    pub fn set_style(&self, style: ProgressStyle) {
        self.pb.set_style(style);
    }
}
