use mutant_protocol::{
    GetCallback, GetEvent, InitCallback, InitProgressEvent, PurgeCallback, PurgeEvent, PutCallback,
    PutEvent, SyncCallback, SyncEvent,
};

use crate::error::Error;

pub async fn invoke_put_callback(
    callback: &mut Option<PutCallback>,
    event: PutEvent,
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
    callback: &mut Option<GetCallback>,
    event: GetEvent,
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
    callback: &mut Option<PurgeCallback>,
    event: PurgeEvent,
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
    callback: &mut Option<SyncCallback>,
    event: SyncEvent,
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
