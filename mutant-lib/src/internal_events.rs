use mutant_protocol::{
    GetCallback as ProtocolGetCallback, GetEvent as ProtocolGetEvent,
    InitCallback as ProtocolInitCallback, InitProgressEvent as ProtocolInitProgressEvent,
    PurgeCallback as ProtocolPurgeCallback, PurgeEvent as ProtocolPurgeEvent,
    PutCallback as ProtocolPutCallback, PutEvent as ProtocolPutEvent,
    SyncCallback as ProtocolSyncCallback, SyncEvent as ProtocolSyncEvent,
};

use crate::error::Error;

pub async fn invoke_put_callback(
    callback: &mut Option<ProtocolPutCallback>,
    event: ProtocolPutEvent,
) -> Result<bool, Error> {
    if let Some(cb) = callback {
        match cb(event).await {
            Ok(continue_op) => Ok(continue_op),
            Err(e) => Err(Error::CallbackError(e.to_string())),
        }
    } else {
        Ok(true)
    }
}

pub(crate) async fn invoke_get_callback(
    callback: &mut Option<ProtocolGetCallback>,
    event: ProtocolGetEvent,
) -> Result<bool, Error> {
    if let Some(cb) = callback {
        match cb(event).await {
            Ok(continue_op) => Ok(continue_op),
            Err(e) => Err(Error::CallbackError(e.to_string())),
        }
    } else {
        Ok(true)
    }
}

pub(crate) async fn invoke_purge_callback(
    callback: &mut Option<ProtocolPurgeCallback>,
    event: ProtocolPurgeEvent,
) -> Result<bool, Error> {
    if let Some(cb) = callback {
        match cb(event).await {
            Ok(continue_op) => Ok(continue_op),
            Err(e) => Err(Error::CallbackError(e.to_string())),
        }
    } else {
        Ok(true)
    }
}

pub(crate) async fn invoke_sync_callback(
    callback: &mut Option<ProtocolSyncCallback>,
    event: ProtocolSyncEvent,
) -> Result<bool, Error> {
    if let Some(cb) = callback {
        match cb(event).await {
            Ok(continue_op) => Ok(continue_op),
            Err(e) => Err(Error::CallbackError(e.to_string())),
        }
    } else {
        Ok(true)
    }
}
