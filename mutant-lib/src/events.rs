use crate::error::Error;
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Events emitted during the 'put' (store) operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PutEvent {
    /// Indicates the start of the reservation process for a batch of pads.
    ReservingPads { count: u64 },
    /// Indicates that the data upload process is starting.
    StartingUpload { total_bytes: u64 },
    /// Reports byte-level progress of the overall data upload.
    UploadProgress {
        bytes_written: u64,
        total_bytes: u64,
    },
    /// Indicates that a single *new* pad was successfully created on the network (before verification).
    PadCreateSuccess { index: u64, total: u64 },
    /// Indicates that a single pad has been successfully verified and confirmed.
    PadConfirmed { current: u64, total: u64 },
    /// Indicates the entire store operation (allocation, write, metadata update) is complete.
    StoreComplete,
}

/// Type alias for the callback function used during the 'put' operation.
/// Uses `async FnMut` which requires the `async_fn_in_trait` feature.
/// It returns a `BoxFuture` to handle the async nature in a trait object.
pub type PutCallback =
    Box<dyn FnMut(PutEvent) -> BoxFuture<'static, Result<bool, Error>> + Send + Sync>;

#[inline]
pub async fn invoke_callback(
    callback: &mut Option<PutCallback>,
    event: PutEvent,
) -> Result<(), Error> {
    if let Some(cb) = callback {
        let fut = cb(event);
        match fut.await {
            Ok(true) => Ok(()),
            Ok(false) => Err(Error::OperationCancelled),
            Err(e) => Err(e),
        }
    } else {
        Ok(())
    }
}

/// Events emitted during the library initialization, primarily for first-run setup.
#[derive(Debug, Clone)]
pub enum InitProgressEvent {
    /// Starting the initialization process.
    Starting { total_steps: u64 },
    /// Progress update for a specific step.
    Step { step: u64, message: String },
    /// Initialization completed successfully.
    Complete { message: String },
    /// Initialization failed.
    Failed { error_msg: String },
    /// An existing configuration was found and loaded (skipping first-run).
    ExistingLoaded { message: String },
    /// Prompt the user whether to create the remote index.
    PromptCreateRemoteIndex,
}

/// Type alias for the callback function used during library initialization.
/// The callback returns a simple Result to indicate success or failure, but
/// doesn't need to return a boolean like PutCallback.
pub type InitCallback =
    Arc<dyn Fn(InitProgressEvent) -> BoxFuture<'static, Result<Option<bool>, Error>> + Send + Sync>;

#[inline]
pub(crate) async fn invoke_init_callback(
    callback: &mut Option<InitCallback>,
    event: InitProgressEvent,
) -> Result<(), Error> {
    if let Some(cb) = callback {
        let fut = cb(event);
        match fut.await {
            Ok(Some(true)) | Ok(None) => Ok(()), // Confirmed or no decision needed = Success for invoker
            Ok(Some(false)) => Err(Error::OperationCancelled), // Declined = Cancellation
            Err(e) => Err(e),                    // Propagate callback error
        }
    } else {
        Ok(()) // No callback, always success
    }
}

/// Events emitted during the `MutAnt::fetch` process.
#[derive(Debug, Clone)]
pub enum GetEvent {
    /// Indicates the start of the download process, providing the total size.
    StartingDownload { total_bytes: u64 },
    /// Reports the progress of the download.
    DownloadProgress { bytes_read: u64, total_bytes: u64 },
    /// Indicates that the download has finished successfully.
    DownloadFinished,
}

/// Callback type for Get/Fetch progress updates.
/// The callback returns a simple Result to indicate success or failure.
pub type GetCallback =
    Box<dyn FnMut(GetEvent) -> BoxFuture<'static, Result<(), Error>> + Send + Sync>;

// Helper function for invoking the Get callback if it exists.
// Note: We might need a distinct invoke function if the return type differs significantly,
// but since both InitCallback and GetCallback return Result<(), Error>, we might be able to reuse or generalize.
// For now, let's assume invoke_init_callback logic works (simple propagation).
#[inline]
pub(crate) async fn invoke_get_callback(
    callback: &mut Option<GetCallback>,
    event: GetEvent,
) -> Result<(), Error> {
    if let Some(cb) = callback {
        let fut = cb(event);
        fut.await
    } else {
        Ok(())
    }
}
