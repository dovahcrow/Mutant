use crate::error::Error;
use std::future::Future;
use std::pin::Pin;

pub type PutCallback = Box<
    dyn Fn(PutEvent) -> Pin<Box<dyn Future<Output = Result<bool, Error>> + Send + Sync>>
        + Send
        + Sync,
>;

pub type GetCallback = Box<
    dyn Fn(GetEvent) -> Pin<Box<dyn Future<Output = Result<bool, Error>> + Send + Sync>>
        + Send
        + Sync,
>;

pub type InitCallback = Box<
    dyn Fn(
            InitProgressEvent,
        ) -> Pin<Box<dyn Future<Output = Result<Option<bool>, Error>> + Send + Sync>>
        + Send
        + Sync,
>;

pub type PurgeCallback = Box<
    dyn Fn(PurgeEvent) -> Pin<Box<dyn Future<Output = Result<bool, Error>> + Send + Sync>>
        + Send
        + Sync,
>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PutEvent {
    Starting {
        total_chunks: usize,
        initial_written_count: usize,
        initial_confirmed_count: usize,
    },

    PadReserved {
        count: usize,
    },

    ChunkWritten {
        chunk_index: usize,
    },

    ChunkConfirmed {
        chunk_index: usize,
    },

    SavingIndex,

    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GetEvent {
    Starting { total_chunks: usize },

    ChunkFetched { chunk_index: usize },

    Reassembling,

    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitProgressEvent {
    Starting { total_steps: u64 },

    Step { step: u64, message: String },

    PromptCreateRemoteIndex,

    Failed { error_msg: String },

    Complete { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PurgeEvent {
    Starting {
        total_count: usize,
    },

    PadProcessed,

    Complete {
        verified_count: usize,
        failed_count: usize,
    },
}

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
    } else {
        if matches!(event, InitProgressEvent::PromptCreateRemoteIndex) {
            Ok(Some(false))
        } else {
            Ok(None)
        }
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
