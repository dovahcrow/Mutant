use crate::internal_error::Error;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
/// Callback type used during `put` operations to report progress and allow cancellation.
///
/// The callback receives `PutEvent` variants and returns a `Future` that resolves to:
/// - `Ok(true)`: Continue the operation.
/// - `Ok(false)`: Cancel the operation (results in `Error::OperationCancelled`).
/// - `Err(e)`: Propagate an error from the callback.
pub type PutCallback = Arc<
    dyn Fn(PutEvent) -> Pin<Box<dyn Future<Output = Result<bool, Error>> + Send + Sync>>
        + Send
        + Sync,
>;

/// Callback type used during `get` operations to report progress and allow cancellation.
///
/// The callback receives `GetEvent` variants and returns a `Future` that resolves to:
/// - `Ok(true)`: Continue the operation.
/// - `Ok(false)`: Cancel the operation (results in `Error::OperationCancelled`).
/// - `Err(e)`: Propagate an error from the callback.
pub type GetCallback = Arc<
    dyn Fn(GetEvent) -> Pin<Box<dyn Future<Output = Result<bool, Error>> + Send + Sync>>
        + Send
        + Sync,
>;

/// Callback type used during initialization (`init`) operations to report progress
/// and handle interactive prompts.
///
/// The callback receives `InitProgressEvent` variants and returns a `Future` that resolves to:
/// - `Ok(Some(true))`: User confirmed action (e.g., create remote index).
/// - `Ok(Some(false))`: User denied action.
/// - `Ok(None)`: Event acknowledged, no specific user action required.
/// - `Err(e)`: Propagate an error from the callback.
pub type InitCallback = Box<
    dyn Fn(
            InitProgressEvent,
        ) -> Pin<Box<dyn Future<Output = Result<Option<bool>, Error>> + Send + Sync>>
        + Send
        + Sync,
>;

/// Callback type used during `purge` operations to report progress and allow cancellation.
///
/// The callback receives `PurgeEvent` variants and returns a `Future` that resolves to:
/// - `Ok(true)`: Continue the operation.
/// - `Ok(false)`: Cancel the operation (results in `Error::OperationCancelled`).
/// - `Err(e)`: Propagate an error from the callback.
pub type PurgeCallback = Arc<
    dyn Fn(PurgeEvent) -> Pin<Box<dyn Future<Output = Result<bool, Error>> + Send + Sync>>
        + Send
        + Sync,
>;

/// Events emitted during a `put` operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PutEvent {
    /// Indicates the start of the `put` operation.
    Starting {
        /// Total number of data chunks to be processed.
        total_chunks: usize,
        /// Number of chunks already found written in storage from a previous attempt.
        initial_written_count: usize,
        /// Number of chunks already found confirmed in storage from a previous attempt.
        initial_confirmed_count: usize,
        /// Number of chunks to reserve
        chunks_to_reserve: usize,
    },

    /// Indicates that a batch of storage pads has been reserved.
    PadReserved,

    /// Indicates that a specific data chunk has been written to storage.
    PadsWritten,

    /// Indicates that a specific data chunk has been confirmed (e.g., replicated).
    PadsConfirmed,

    /// Indicates that the `put` operation has completed successfully.
    Complete,
}

/// Events emitted during a `get` operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GetEvent {
    /// Indicates the start of chunk fetching.
    Starting {
        /// Total number of data chunks to be fetched (including index).
        total_chunks: usize,
    },

    /// Indicates that a specific data chunk has been fetched from storage.
    PadsFetched,

    /// Indicates that the `get` operation has completed successfully.
    Complete,
}

/// Events emitted during an `init` (initialization) operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitProgressEvent {
    /// Indicates the start of the initialization process.
    Starting {
        /// An estimated total number of steps for the initialization.
        total_steps: u64,
    },

    /// Reports progress on a specific step during initialization.
    Step {
        /// The current step number.
        step: u64,
        /// A message describing the current step.
        message: String,
    },

    /// Indicates that user confirmation is required to create a remote index.
    /// The `InitCallback` should return `Ok(Some(true))` to proceed or `Ok(Some(false))` to skip.
    PromptCreateRemoteIndex,

    /// Indicates that the initialization process has failed.
    Failed {
        /// A message describing the failure.
        error_msg: String,
    },

    /// Indicates that the initialization process has completed successfully.
    Complete {
        /// A final message summarizing the outcome.
        message: String,
    },
}

/// Events emitted during a `purge` operation (storage cleanup/verification).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PurgeEvent {
    /// Indicates the start of the `purge` operation.
    Starting {
        /// Total number of pads to be processed.
        total_count: usize,
    },

    /// Indicates that a single pad has been processed (verified or marked for cleanup).
    PadProcessed,

    /// Indicates that the `purge` operation has completed.
    Complete {
        /// Number of pads successfully verified.
        verified_count: usize,
        /// Number of pads that failed verification or encountered errors.
        failed_count: usize,
    },
}

/// Helper function to asynchronously invoke an optional `PutCallback`.
///
/// If the callback is `Some`, it is called with the given `event`.
/// If the callback returns `Ok(false)`, this function returns `Ok(false)` to signal cancellation.
/// If the callback returns `Err(e)`, the error is propagated.
/// If the callback is `None`, this function returns `Ok(true)`.
pub async fn invoke_put_callback(
    callback: &mut Option<PutCallback>,
    event: PutEvent,
) -> Result<bool, Error> {
    if let Some(cb) = callback {
        match cb(event).await {
            Ok(continue_op) => Ok(continue_op),
            Err(e) => Err(e),
        }
    } else {
        Ok(true)
    }
}

pub(crate) async fn invoke_get_callback(
    callback: &mut Option<GetCallback>,
    event: GetEvent,
) -> Result<bool, Error> {
    if let Some(cb) = callback {
        match cb(event).await {
            Ok(continue_op) => Ok(continue_op),
            Err(e) => Err(e),
        }
    } else {
        Ok(true)
    }
}

pub(crate) async fn invoke_init_callback(
    callback: &mut Option<InitCallback>,
    event: InitProgressEvent,
) -> Result<Option<bool>, Error> {
    if let Some(cb) = callback {
        match cb(event).await {
            Ok(response) => Ok(response),
            Err(e) => Err(e),
        }
    } else if matches!(event, InitProgressEvent::PromptCreateRemoteIndex) {
        Ok(Some(false))
    } else {
        Ok(None)
    }
}

pub(crate) async fn invoke_purge_callback(
    callback: &mut Option<PurgeCallback>,
    event: PurgeEvent,
) -> Result<bool, Error> {
    if let Some(cb) = callback {
        match cb(event).await {
            Ok(continue_op) => Ok(continue_op),
            Err(e) => Err(e),
        }
    } else {
        Ok(true)
    }
}
