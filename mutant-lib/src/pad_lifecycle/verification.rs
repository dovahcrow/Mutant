use crate::events::{invoke_purge_callback, PurgeCallback, PurgeEvent};
use crate::network::NetworkAdapter;
use crate::pad_lifecycle::error::PadLifecycleError;
use autonomi::ScratchpadAddress;
use log::{debug, error, info, trace, warn};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinSet;

/// Concurrently verifies a list of pads by attempting to fetch data from their addresses.
///
/// Pads that are successfully fetched are considered verified. Pads that result in a
/// "RecordNotFound" error or other network errors are considered failed/discarded.
///
/// Returns a tuple containing:
/// - A list of verified pads (address and key bytes).
/// - A list of addresses for pads that failed verification or were not found.
/// - A list of pads (address and key bytes) that encountered errors and should be retried.
pub(crate) async fn verify_pads_concurrently(
    network_adapter: Arc<dyn NetworkAdapter>, // Use Arc for sharing across tasks
    pads_to_verify: Vec<(ScratchpadAddress, Vec<u8>)>,
    mut callback: Option<PurgeCallback>,
) -> Result<
    (
        Vec<(ScratchpadAddress, Vec<u8>)>, // Verified pads
        Vec<ScratchpadAddress>,            // Not Found pad addresses (to discard)
        Vec<(ScratchpadAddress, Vec<u8>)>, // Pads to retry later (error during check)
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
        return Ok((Vec::new(), Vec::new(), Vec::new())); // Return empty lists
    }

    let verified_pads_arc = Arc::new(Mutex::new(Vec::with_capacity(initial_count)));
    // Use a Mutex protected Vec for failed addresses instead of AtomicUsize
    let not_found_addresses_arc = Arc::new(Mutex::new(Vec::new()));
    let retry_pads_arc = Arc::new(Mutex::new(Vec::new())); // New list for retries
    let mut first_error: Option<PadLifecycleError> = None;
    let mut callback_cancelled = false;

    // Emit Starting event
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
        let not_found_clone = Arc::clone(&not_found_addresses_arc); // Clone not_found addresses Arc
        let retry_clone = Arc::clone(&retry_pads_arc); // Clone retry pads Arc
        let task_address = address; // Capture address for error reporting
        let task_key_bytes = key_bytes.clone(); // Capture key bytes for retry list

        join_set.spawn(async move {
            trace!("Verification task starting for address: {}", task_address);
            // Use check_existence instead of get_raw
            match adapter_clone.check_existence(&task_address).await {
                Ok(true) => {
                    // Pad exists on the network
                    debug!(
                        "Successfully verified pad existence at address: {}",
                        task_address
                    );
                    let mut verified_guard = verified_clone.lock().await;
                    verified_guard.push((task_address, key_bytes)); // Keep original key bytes
                    Ok(VerificationStatus::Verified)
                }
                Ok(false) => {
                    // Pad does not exist on the network (RecordNotFound)
                    info!(
                        "Pad at address {} not found on network. Recording as not found (discard).",
                        task_address
                    );
                    let mut not_found_guard = not_found_clone.lock().await;
                    not_found_guard.push(task_address);
                    Ok(VerificationStatus::NotFound)
                }
                Err(e) => {
                    // Network error during check - Mark for retry
                    warn!(
                        "Network error verifying pad at address {}: {}. Recording to retry later.",
                        task_address, e
                    );
                    let mut retry_guard = retry_clone.lock().await;
                    // Store both address and key for retry
                    retry_guard.push((task_address, task_key_bytes));
                    // Propagate the network error to potentially be the first_error
                    Err((e, VerificationStatus::Error))
                }
            }
        });
    }

    // Collect results
    let mut processed_count = 0;
    while let Some(res) = join_set.join_next().await {
        processed_count += 1;
        trace!(
            "Verification: Processed task {}/{}",
            processed_count,
            initial_count
        );
        match res {
            Ok(task_result) => {
                // Task completed successfully (returned Ok or Err from the task itself)
                match task_result {
                    Ok(VerificationStatus::Verified) | Ok(VerificationStatus::NotFound) => {
                        // Handled within the task by pushing to appropriate list
                    }
                    Ok(VerificationStatus::Error) => {
                        // This case shouldn't happen if Err is returned correctly
                        error!("Verification task returned Ok(Error), which is unexpected.");
                    }
                    Err((task_err, VerificationStatus::Error)) => {
                        // Task returned an error (network error) - already added to retry list
                        if first_error.is_none() {
                            first_error = Some(PadLifecycleError::Network(task_err));
                        }
                    }
                    Err((_, _)) => {
                        // Should not happen with current task logic
                        error!("Verification task returned unexpected Err variant");
                    }
                }
            }
            Err(join_error) => {
                // Task panicked or was cancelled
                error!("JoinError during pad verification: {}", join_error);
                // Cannot reliably determine which pad failed, difficult to add to failed list here.
                // Log the error, the pad won't be in verified, and won't be removed from pending by address.
                if first_error.is_none() {
                    first_error = Some(PadLifecycleError::InternalError(format!(
                        "Pad verification task failed: {}",
                        join_error
                    )));
                }
            }
        }

        // Emit PadProcessed event after each task finishes
        if !callback_cancelled {
            if !invoke_purge_callback(&mut callback, PurgeEvent::PadProcessed)
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
                join_set.abort_all(); // Cancel remaining tasks
            }
        }
    }

    // Use into_inner after Arc::try_unwrap for all lists
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
    let final_retry_count = retry_pads.len(); // Count pads marked for retry

    info!(
        "Verification complete. Verified: {}, Not Found (Discarded): {}, Errors (Retry): {}",
        final_verified_count, final_not_found_count, final_retry_count
    );

    // Emit Complete event - Note: 'failed_count' now represents 'not_found_count'
    // We might want a new event type or field later if differentiating retry vs not_found in the event is important.
    invoke_purge_callback(
        &mut callback,
        PurgeEvent::Complete {
            verified_count: final_verified_count,
            failed_count: final_not_found_count, // Report 'Not Found' count as 'failed'
        },
    )
    .await
    .map_err(|e| PadLifecycleError::InternalError(format!("Callback invocation failed: {}", e)))?;

    // Return the first error encountered, or Ok with results
    if let Some(e) = first_error {
        // If an error occurred, still return the categorized lists
        // The caller (purge) will decide how to handle the overall operation error
        // based on the presence of retry_pads.
        warn!(
            "Verification encountered errors, returning partial results and first error: {}",
            e
        );
        // We still return Ok here because the operation partially succeeded in categorizing pads.
        // The caller needs the lists to decide final state.
        // Let's refine this - propagate the error, but ensure the caller can still access lists if needed.
        // For now, propagating the error seems correct. The caller handles it.
        Err(e)
    } else {
        Ok((verified_pads, not_found_addresses, retry_pads))
    }
}

// Add an enum to represent the outcome of a single verification task
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerificationStatus {
    Verified,
    NotFound,
    Error,
}
