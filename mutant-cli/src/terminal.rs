use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};

// Flag to track if stdin is disabled
static STDIN_DISABLED: AtomicBool = AtomicBool::new(false);

/// A guard that disables stdin while it's in scope and re-enables it when dropped
pub struct StdinGuard {
    _private: (), // Private field to prevent construction outside this module
}

impl StdinGuard {
    /// Create a new guard that disables stdin
    pub fn new() -> Self {
        // Set the flag to indicate stdin is disabled
        STDIN_DISABLED.store(true, Ordering::SeqCst);

        // Flush stdout to ensure any pending output is displayed
        let _ = io::stdout().flush();

        Self { _private: () }
    }
}

impl Drop for StdinGuard {
    fn drop(&mut self) {
        // Re-enable stdin when the guard is dropped
        STDIN_DISABLED.store(false, Ordering::SeqCst);

        // Flush stdout to ensure any pending output is displayed
        let _ = io::stdout().flush();
    }
}

// These functions are kept for future use if needed
#[allow(dead_code)]
/// Check if stdin is currently disabled
pub fn is_stdin_disabled() -> bool {
    STDIN_DISABLED.load(Ordering::SeqCst)
}

#[allow(dead_code)]
/// Disable stdin and return a guard that will re-enable it when dropped
pub fn disable_stdin() -> StdinGuard {
    StdinGuard::new()
}

/// Wrapper for MultiProgress that disables stdin while it exists
pub struct ProgressWithDisabledStdin {
    _stdin_guard: StdinGuard,
    multi_progress: indicatif::MultiProgress,
}

impl ProgressWithDisabledStdin {
    /// Create a new MultiProgress with stdin disabled
    pub fn new() -> Self {
        Self {
            _stdin_guard: StdinGuard::new(),
            multi_progress: indicatif::MultiProgress::new(),
        }
    }

    /// Get a reference to the underlying MultiProgress
    pub fn multi_progress(&self) -> &indicatif::MultiProgress {
        &self.multi_progress
    }
}

impl std::ops::Deref for ProgressWithDisabledStdin {
    type Target = indicatif::MultiProgress;

    fn deref(&self) -> &Self::Target {
        &self.multi_progress
    }
}
