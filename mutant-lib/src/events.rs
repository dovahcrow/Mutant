use crate::error::Error; // Assuming top-level error exists or will exist
use async_trait::async_trait;
use std::future::Future;
use std::pin::Pin;

// --- Callback Types ---
// Define function signature types for callbacks.
// Use Pin<Box<dyn Future>> for async operations within callbacks.
// Add Send + Sync bounds as callbacks might be called from different threads.

/// Callback for reporting progress during data storage (put/update) operations.
/// The callback receives a PutEvent and should return true to continue, false to cancel.
pub type PutCallback = Box<
    dyn Fn(PutEvent) -> Pin<Box<dyn Future<Output = Result<bool, Error>> + Send + Sync>>
        + Send
        + Sync,
>;

/// Callback for reporting progress during data retrieval (get) operations.
/// The callback receives a GetEvent and should return true to continue, false to cancel.
pub type GetCallback = Box<
    dyn Fn(GetEvent) -> Pin<Box<dyn Future<Output = Result<bool, Error>> + Send + Sync>>
        + Send
        + Sync,
>;

/// Callback for reporting progress during library initialization.
/// The callback receives an InitProgressEvent and should return true to continue, false to cancel.
/// The boolean return for PromptCreateRemoteIndex indicates user choice (true=create, false=don't create).
pub type InitCallback = Box<
    dyn Fn(InitProgressEvent) -> Pin<Box<dyn Future<Output = Result<Option<bool>, Error>> + Send + Sync>>
        + Send
        + Sync,
>;

/// Callback for reporting progress during pad purging (verification).
/// The callback receives a PurgeEvent and should return true to continue, false to cancel.
pub type PurgeCallback = Box<
    dyn Fn(PurgeEvent) -> Pin<Box<dyn Future<Output = Result<bool, Error>> + Send + Sync>>
        + Send
        + Sync,
>;


// --- Event Enums ---

/// Events reported during data storage (put/update) operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PutEvent {
    /// Starting the operation. Provides total chunks to be written.
    Starting { total_chunks: usize },
    /// A single chunk has been successfully written to its pad.
    ChunkWritten { chunk_index: usize },
    /// The master index is being saved after chunk writes.
    SavingIndex,
    /// The operation completed successfully.
    Complete,
}

/// Events reported during data retrieval (get) operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GetEvent {
    /// Starting the operation. Provides total chunks to be fetched.
    Starting { total_chunks: usize },
    /// A single chunk has been successfully fetched from its pad.
    ChunkFetched { chunk_index: usize },
    /// The data is being reassembled from chunks.
    Reassembling,
    /// The operation completed successfully.
    Complete,
}

/// Events reported during library initialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitProgressEvent {
    /// Initialization is starting. Provides the total number of steps.
    Starting { total_steps: u64 },
    /// Progressing to a specific step.
    Step { step: u64, message: String },
    /// Master index not found remotely, prompt user whether to create it.
    /// Callback should return Ok(Some(true)) to create, Ok(Some(false)) to skip, Ok(None) or Err to abort.
    PromptCreateRemoteIndex,
    /// Initialization failed at some step.
    Failed { error_msg: String },
    /// Initialization completed successfully.
    Complete { message: String },
}


/// Events reported during pad purging (verification).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PurgeEvent {
    /// Purge operation is starting. Provides the total number of pads to check.
    Starting { total_count: usize },
    /// One pad has been processed (either verified or failed).
    PadProcessed,
    /// Purge operation completed. Provides counts of verified and failed pads.
    Complete {
        verified_count: usize,
        failed_count: usize,
    },
}


// --- Helper Functions for Invoking Callbacks ---

/// Helper to invoke an optional PutCallback safely.
pub(crate) async fn invoke_put_callback(
    callback: &mut Option<PutCallback>,
    event: PutEvent,
) -> Result<bool, Error> {
    if let Some(cb) = callback {
        // Call the async closure and await its result
        match cb(event).await {
            Ok(continue_op) => Ok(continue_op),
            Err(e) => {
                // Propagate the error returned by the callback
                Err(e)
            }
        }
    } else {
        Ok(true) // No callback, always continue
    }
}

/// Helper to invoke an optional GetCallback safely.
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

/// Helper to invoke an optional InitCallback safely.
/// Returns Ok(Some(bool)) for PromptCreateRemoteIndex response, Ok(None) otherwise, or Err.
pub(crate) async fn invoke_init_callback(
    callback: &mut Option<InitCallback>,
    event: InitProgressEvent,
) -> Result<Option<bool>, Error> {
     if let Some(cb) = callback {
        match cb(event).await {
            Ok(response) => Ok(response), // Propagate user response or None
            Err(e) => Err(e),
        }
    } else {
        // If no callback, default behavior for PromptCreateRemoteIndex is 'false' (don't create)
        if matches!(event, InitProgressEvent::PromptCreateRemoteIndex) {
             Ok(Some(false))
        } else {
            Ok(None) // No response needed for other events
        }
    }
}

/// Helper to invoke an optional PurgeCallback safely.
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