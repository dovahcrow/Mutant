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
pub(crate) async fn verify_pads_concurrently(
    network_adapter: Arc<dyn NetworkAdapter>, // Use Arc for sharing across tasks
    pads_to_verify: Vec<(ScratchpadAddress, Vec<u8>)>,
    mut callback: Option<PurgeCallback>,
) -> Result<
    (
        Vec<(ScratchpadAddress, Vec<u8>)>, // Verified pads
        Vec<ScratchpadAddress>,            // Failed/Not Found pad addresses
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
        return Ok((Vec::new(), Vec::new())); // Return empty lists
    }

    let verified_pads_arc = Arc::new(Mutex::new(Vec::with_capacity(initial_count)));
    // Use a Mutex protected Vec for failed addresses instead of AtomicUsize
    let failed_addresses_arc = Arc::new(Mutex::new(Vec::new()));
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
        let failed_clone = Arc::clone(&failed_addresses_arc); // Clone failed addresses Arc
        let task_address = address; // Capture address for error reporting

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
                    Ok(()) // Indicate success
                }
                Ok(false) => {
                    // Pad does not exist on the network
                    info!(
                        "Pad at address {} not found on network. Recording as failed.",
                        task_address
                    );
                    let mut failed_guard = failed_clone.lock().await;
                    failed_guard.push(task_address);
                    Ok(()) // Indicate success (task completed, pad handled as failed)
                }
                Err(e) => {
                    // Network error during check
                    warn!(
                        "Network error verifying pad at address {}: {}. Recording as failed.",
                        task_address, e
                    );
                    let mut failed_guard = failed_clone.lock().await;
                    failed_guard.push(task_address);
                    Err(e) // Propagate the network error
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
                if let Err(task_err) = task_result {
                    // Task returned an error (e.g., network error)
                    if first_error.is_none() {
                        first_error = Some(PadLifecycleError::Network(task_err));
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

    // Use into_inner after Arc::try_unwrap for both lists
    let verified_pads = Arc::try_unwrap(verified_pads_arc)
        .map_err(|_| {
            PadLifecycleError::InternalError("Failed to unwrap verified pads Arc".to_string())
        })?
        .into_inner();
    let failed_addresses = Arc::try_unwrap(failed_addresses_arc)
        .map_err(|_| {
            PadLifecycleError::InternalError("Failed to unwrap failed addresses Arc".to_string())
        })?
        .into_inner();

    let final_verified_count = verified_pads.len();
    let final_failed_count = failed_addresses.len();

    info!(
        "Verification complete. Verified: {}, Failed/Not Found: {}",
        final_verified_count, final_failed_count
    );

    // Emit Complete event
    invoke_purge_callback(
        &mut callback,
        PurgeEvent::Complete {
            verified_count: final_verified_count,
            failed_count: final_failed_count,
        },
    )
    .await
    .map_err(|e| PadLifecycleError::InternalError(format!("Callback invocation failed: {}", e)))?;

    // Return the first error encountered, or Ok with results
    if let Some(e) = first_error {
        Err(e)
    } else {
        Ok((verified_pads, failed_addresses))
    }
}
