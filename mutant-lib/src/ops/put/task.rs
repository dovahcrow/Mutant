use crate::error::Error;
use crate::index::{PadInfo, PadStatus};
use crate::internal_events::invoke_put_callback;
use crate::network::NetworkError;
use crate::ops::worker::AsyncTask;
use crate::ops::MAX_CONFIRMATION_DURATION;
use async_trait::async_trait;
use log::{debug, error, info, warn};
use mutant_protocol::PutEvent;
use std::{sync::Arc, time::Duration};
use tokio::time::Instant;

use super::context::PutTaskContext;
use super::super::{
    DATA_ENCODING_PRIVATE_DATA, DATA_ENCODING_PUBLIC_DATA, DATA_ENCODING_PUBLIC_INDEX,
    PAD_RECYCLING_RETRIES,
};

#[derive(Clone)]
pub struct PutTaskProcessor {
    pub context: Arc<PutTaskContext>,
}

impl PutTaskProcessor {
    pub fn new(context: Arc<PutTaskContext>) -> Self {
        Self { context }
    }
}

#[async_trait]
impl AsyncTask<PadInfo, PutTaskContext, autonomi::Client, (), Error>
    for PutTaskProcessor
{
    type ItemId = usize;

    async fn process(
        &self,
        worker_id: usize,
        client: &autonomi::Client,
        pad: PadInfo,
    ) -> Result<(Self::ItemId, ()), (Error, PadInfo)> {
        let mut pad_state = pad.clone();
        let current_pad_address = pad_state.address;
        let initial_status = pad_state.status;
        let mut put_succeeded = false;
        let is_public = self.context.base_context.public;

        let should_put = match initial_status {
            PadStatus::Generated | PadStatus::Free => true,
            PadStatus::Written => false,
            PadStatus::Confirmed => {
                return Ok((pad_state.chunk_index, ()));
            }
        };

        if should_put {
            // Use the is_index_pad flag from the context to determine if this is an index pad
            let is_index_pad = is_public && self.context.base_context.is_index_pad;

            // Check if we have a preserved index pad that we should use instead
            if is_index_pad && self.context.base_context.preserved_index_pad.is_some() {
                // If we have a preserved index pad, we should use its address instead
                if let Some(preserved_pad) = &self.context.base_context.preserved_index_pad {
                    info!(
                        "Worker {} using preserved index pad address {} instead of {}",
                        worker_id, preserved_pad.address, pad_state.address
                    );
                    pad_state.address = preserved_pad.address;
                }
            }

            let data_encoding = if is_public {
                if is_index_pad {
                    debug!(
                        "Using PUBLIC_INDEX encoding for index pad {} (chunk_index={})",
                        pad_state.address, pad_state.chunk_index
                    );
                    DATA_ENCODING_PUBLIC_INDEX
                } else {
                    debug!(
                        "Using PUBLIC_DATA encoding for data pad {} (chunk_index={})",
                        pad_state.address, pad_state.chunk_index
                    );
                    DATA_ENCODING_PUBLIC_DATA
                }
            } else {
                debug!(
                    "Using PRIVATE_DATA encoding for pad {} (chunk_index={})",
                    pad_state.address, pad_state.chunk_index
                );
                DATA_ENCODING_PRIVATE_DATA
            };

            let chunk_index = pad_state.chunk_index;
            let range = self
                .context
                .base_context
                .chunk_ranges
                .get(chunk_index)
                .ok_or_else(|| {
                    (
                        Error::Internal(format!(
                            "Invalid chunk index {} for key {}",
                            chunk_index, self.context.base_context.name
                        )),
                        pad_state.clone(),
                    )
                })?;
            let chunk_data = self
                .context
                .base_context
                .data
                .get(range.clone())
                .ok_or_else(|| {
                    (
                        Error::Internal(format!(
                            "Data range {:?} out of bounds for key {}",
                            range, self.context.base_context.name
                        )),
                        pad_state.clone(),
                    )
                })?;

            let max_put_retries = PAD_RECYCLING_RETRIES;
            let mut last_put_error: Option<Error> = None;
            for attempt in 1..=max_put_retries {
                let put_result = self
                    .context
                    .base_context
                    .network
                    .put(client, &pad_state, chunk_data, data_encoding, is_public)
                    .await;

                match put_result {
                    Ok(_) => {
                        // Check if this was a Generated pad that needs a PadReserved event
                        let was_generated = initial_status == PadStatus::Generated;

                        pad_state.status = PadStatus::Written;
                        match self
                            .context
                            .base_context
                            .index
                            .write()
                            .await
                            .update_pad_status(
                                &self.context.base_context.name,
                                &current_pad_address,
                                PadStatus::Written,
                                None,
                            ) {
                            Ok(updated_pad) => pad_state = updated_pad,
                            Err(e) => return Err((e, pad_state.clone())),
                        }

                        // If the pad was in Generated status, send PadReserved event
                        if was_generated {
                            info!(
                                "Worker {} sending PadReserved event for pad {} (chunk {})",
                                worker_id, current_pad_address, pad_state.chunk_index
                            );
                            invoke_put_callback(&self.context.put_callback, PutEvent::PadReserved)
                                .await
                                .map_err(|e| (e, pad_state.clone()))?;
                        }

                        put_succeeded = true;
                        last_put_error = None;
                        break;
                    }
                    Err(e) => {
                        warn!(
                            "Worker {} failed put attempt {}/{} for pad {} (chunk {}): {}. Retrying...",
                            worker_id, attempt, max_put_retries, current_pad_address, pad_state.chunk_index, e
                        );
                        last_put_error = Some(Error::Network(e));
                        if attempt < max_put_retries {
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                    }
                }
            }

            if !put_succeeded {
                return Err((
                    last_put_error.unwrap_or_else(|| {
                        Error::Internal(format!(
                            "Put failed for pad {} after {} retries with unknown error",
                            current_pad_address, max_put_retries
                        ))
                    }),
                    pad_state,
                ));
            }

            invoke_put_callback(&self.context.put_callback, PutEvent::PadsWritten)
                .await
                .map_err(|e| (e, pad_state.clone()))?;
        } else {
            put_succeeded = true;
            pad_state = pad.clone();
        }

        if put_succeeded && !*self.context.no_verify {
            let confirmation_start = Instant::now();
            let max_duration = MAX_CONFIRMATION_DURATION;
            let mut confirmation_succeeded = false;

            while confirmation_start.elapsed() < max_duration {
                let owned_key;
                let secret_key_ref = if is_public {
                    None
                } else {
                    owned_key = pad_state.secret_key();
                    Some(&owned_key)
                };
                match self
                    .context
                    .base_context
                    .network
                    .get(client, &current_pad_address, secret_key_ref)
                    .await
                {
                    Ok(get_result) => {
                        let checksum_match = pad.checksum == PadInfo::checksum(&get_result.data);
                        let counter_match = pad.last_known_counter == get_result.counter;
                        let size_match = pad.size == get_result.data.len();
                        if checksum_match && counter_match && size_match {
                            pad_state.status = PadStatus::Confirmed;
                            match self
                                .context
                                .base_context
                                .index
                                .write()
                                .await
                                .update_pad_status(
                                    &self.context.base_context.name,
                                    &current_pad_address,
                                    PadStatus::Confirmed,
                                    None,
                                ) {
                                Ok(final_pad) => {
                                    confirmation_succeeded = true;
                                    pad_state = final_pad;
                                    break;
                                }
                                Err(e) => {
                                    warn!("Worker {} failed to update index status to Confirmed for pad {}: {}. Retrying confirmation...", worker_id, current_pad_address, e);
                                }
                            }
                        }
                    }
                    Err(NetworkError::GetError(ant_networking::GetRecordError::RecordNotFound)) => {
                        debug!(
                            "Worker {} confirming pad {}, not found yet. Retrying...",
                            worker_id, current_pad_address
                        );
                    }
                    Err(e) => {
                        warn!(
                            "Worker {} encountered network error while confirming pad {}: {}. Retrying confirmation...",
                            worker_id, current_pad_address, e
                        );
                    }
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }

            if !confirmation_succeeded {
                error!(
                    "Worker {} failed to confirm pad {} within {:?}. Returning error.",
                    worker_id, current_pad_address, max_duration
                );
                return Err((
                    Error::Internal(format!("Confirmation timeout: {}", current_pad_address)),
                    pad_state,
                ));
            }

            invoke_put_callback(&self.context.put_callback, PutEvent::PadsConfirmed)
                .await
                .map_err(|e| (e, pad_state.clone()))?;
        }

        Ok((pad_state.chunk_index, ()))
    }
}
