use crate::error::Error;
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};

/// Events emitted during the 'put' (store) operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PutEvent {
    /// Indicates that the allocator needs to reserve new scratchpads.
    ReservingScratchpads { needed: u64 },
    /// Indicates the start of the reservation process for a batch of pads.
    ReservingPads { count: u64 },
    /// Reports the result of reserving a single pad within a batch.
    PadReserved {
        index: u64,
        total: u64,
        result: Result<(), String>, // Ok(()) on success, Err(error_string) on failure
    },
    /// Prompts the caller (e.g., the binary) to confirm the reservation.
    /// The callback should return `Ok(true)` to proceed, `Ok(false)` to cancel.
    ConfirmReservation {
        needed: u64,
        data_size: u64,
        total_space: u64,
        free_space: u64,
        current_scratchpads: usize,
        estimated_cost: Option<String>,
    },
    /// Reports progress during scratchpad reservation.
    ReservationProgress { current: u64, total: u64 },
    /// Indicates that the data upload process is starting.
    StartingUpload { total_bytes: u64 },
    /// Reports progress during data upload.
    UploadProgress {
        bytes_written: u64,
        total_bytes: u64,
    },
    /// Indicates that the data upload process has finished.
    UploadFinished,
    /// Indicates the entire store operation (allocation, write, metadata update) is complete.
    StoreComplete,
    /// Indicates that a single scratchpad's worth of data has been successfully uploaded.
    ScratchpadUploadComplete { index: u64, total: u64 },
    /// Indicates that a single scratchpad has been fully committed/finalized after upload.
    ScratchpadCommitComplete { index: u64, total: u64 },
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
}

/// Type alias for the callback function used during library initialization.
/// The callback returns a simple Result to indicate success or failure, but
/// doesn't need to return a boolean like PutCallback.
pub type InitCallback =
    Box<dyn FnMut(InitProgressEvent) -> BoxFuture<'static, Result<(), Error>> + Send + Sync>;

#[inline]
pub(crate) async fn invoke_init_callback(
    callback: &mut Option<InitCallback>,
    event: InitProgressEvent,
) -> Result<(), Error> {
    if let Some(cb) = callback {
        let fut = cb(event);
        fut.await
    } else {
        Ok(())
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
