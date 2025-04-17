use crate::events::{invoke_purge_callback, PurgeCallback, PurgeEvent};
use crate::network::NetworkAdapter;
use crate::pad_lifecycle::error::PadLifecycleError;
use autonomi::ScratchpadAddress;
use log::{debug, error, info, trace, warn};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinSet;

pub(crate) async fn verify_pads_concurrently(
    network_adapter: Arc<dyn NetworkAdapter>,
    pads_to_verify: Vec<(ScratchpadAddress, Vec<u8>)>,
    mut callback: Option<PurgeCallback>,
) -> Result<
    (
        Vec<(ScratchpadAddress, Vec<u8>)>,
        Vec<ScratchpadAddress>,
        Vec<(ScratchpadAddress, Vec<u8>)>,
    ),
    PadLifecycleError,
> {
    let initial_count = pads_to_verify.len();
    info!(
        "Verification: Starting concurrent check for {} pads.",
        initial_count
    );

    if initial_count == 0 {
        debug!("Verification: No pads to verify.");
        invoke_purge_callback(&mut callback, PurgeEvent::Starting { total_count: 0 })
            .await
            .map_err(|e| {
                PadLifecycleError::InternalError(format!("Callback invocation failed: {}", e))
            })?;
        invoke_purge_callback(
            &mut callback,
            PurgeEvent::Complete {
                verified_count: 0,
                failed_count: 0,
            },
        )
        .await
        .map_err(|e| {
            PadLifecycleError::InternalError(format!("Callback invocation failed: {}", e))
        })?;
        return Ok((Vec::new(), Vec::new(), Vec::new()));
    }

    let verified_pads_arc = Arc::new(Mutex::new(Vec::with_capacity(initial_count)));

    let not_found_addresses_arc = Arc::new(Mutex::new(Vec::new()));
    let retry_pads_arc = Arc::new(Mutex::new(Vec::new()));
    let mut first_error: Option<PadLifecycleError> = None;
    let mut callback_cancelled = false;

    invoke_purge_callback(
        &mut callback,
        PurgeEvent::Starting {
            total_count: initial_count,
        },
    )
    .await
    .map_err(|e| PadLifecycleError::InternalError(format!("Callback invocation failed: {}", e)))?;

    let mut join_set = JoinSet::new();

    for (address, key_bytes) in pads_to_verify {
        let adapter_clone = Arc::clone(&network_adapter);
        let verified_clone = Arc::clone(&verified_pads_arc);
        let not_found_clone = Arc::clone(&not_found_addresses_arc);
        let retry_clone = Arc::clone(&retry_pads_arc);
        let task_address = address;
        let task_key_bytes = key_bytes.clone();

        join_set.spawn(async move {
            trace!("Verification task starting for address: {}", task_address);

            match adapter_clone.check_existence(&task_address).await {
                Ok(true) => {
                    debug!(
                        "Successfully verified pad existence at address: {}",
                        task_address
                    );
                    let mut verified_guard = verified_clone.lock().await;
                    verified_guard.push((task_address, key_bytes));
                    Ok(VerificationStatus::Verified)
                }
                Ok(false) => {
                    info!(
                        "Pad at address {} not found on network. Recording as not found (discard).",
                        task_address
                    );
                    let mut not_found_guard = not_found_clone.lock().await;
                    not_found_guard.push(task_address);
                    Ok(VerificationStatus::NotFound)
                }
                Err(e) => {
                    warn!(
                        "Network error verifying pad at address {}: {}. Recording to retry later.",
                        task_address, e
                    );
                    let mut retry_guard = retry_clone.lock().await;

                    retry_guard.push((task_address, task_key_bytes));

                    Err((e, VerificationStatus::Error))
                }
            }
        });
    }

    let mut processed_count = 0;
    while let Some(res) = join_set.join_next().await {
        processed_count += 1;
        trace!(
            "Verification: Processed task {}/{}",
            processed_count,
            initial_count
        );
        match res {
            Ok(task_result) => match task_result {
                Ok(VerificationStatus::Verified) | Ok(VerificationStatus::NotFound) => {}
                Ok(VerificationStatus::Error) => {
                    error!("Verification task returned Ok(Error), which is unexpected.");
                }
                Err((task_err, VerificationStatus::Error)) => {
                    if first_error.is_none() {
                        first_error = Some(PadLifecycleError::Network(task_err));
                    }
                }
                Err((_, _)) => {
                    error!("Verification task returned unexpected Err variant");
                }
            },
            Err(join_error) => {
                error!("JoinError during pad verification: {}", join_error);

                if first_error.is_none() {
                    first_error = Some(PadLifecycleError::InternalError(format!(
                        "Pad verification task failed: {}",
                        join_error
                    )));
                }
            }
        }

        if !callback_cancelled
            && !invoke_purge_callback(&mut callback, PurgeEvent::PadProcessed)
                .await
                .map_err(|e| {
                    PadLifecycleError::InternalError(format!("Callback invocation failed: {}", e))
                })?
        {
            warn!("Verification cancelled by callback.");
            callback_cancelled = true;
            if first_error.is_none() {
                first_error = Some(PadLifecycleError::OperationCancelled);
            }
            join_set.abort_all();
        }
    }

    let verified_pads = Arc::try_unwrap(verified_pads_arc)
        .map_err(|_| {
            PadLifecycleError::InternalError("Failed to unwrap verified pads Arc".to_string())
        })?
        .into_inner();
    let not_found_addresses = Arc::try_unwrap(not_found_addresses_arc)
        .map_err(|_| {
            PadLifecycleError::InternalError("Failed to unwrap not_found addresses Arc".to_string())
        })?
        .into_inner();
    let retry_pads = Arc::try_unwrap(retry_pads_arc)
        .map_err(|_| {
            PadLifecycleError::InternalError("Failed to unwrap retry pads Arc".to_string())
        })?
        .into_inner();

    let final_verified_count = verified_pads.len();
    let final_not_found_count = not_found_addresses.len();
    let final_retry_count = retry_pads.len();

    info!(
        "Verification complete. Verified: {}, Not Found (Discarded): {}, Errors (Retry): {}",
        final_verified_count, final_not_found_count, final_retry_count
    );

    invoke_purge_callback(
        &mut callback,
        PurgeEvent::Complete {
            verified_count: final_verified_count,
            failed_count: final_not_found_count,
        },
    )
    .await
    .map_err(|e| PadLifecycleError::InternalError(format!("Callback invocation failed: {}", e)))?;

    if let Some(e) = first_error {
        warn!(
            "Verification encountered errors, returning partial results and first error: {}",
            e
        );

        Err(e)
    } else {
        Ok((verified_pads, not_found_addresses, retry_pads))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerificationStatus {
    Verified,
    NotFound,
    Error,
}
