use crate::events::{invoke_purge_callback, PurgeCallback, PurgeEvent};
use crate::network::NetworkAdapter;
use crate::pad_lifecycle::error::PadLifecycleError;
use autonomi::ScratchpadAddress;
use log::{debug, error, info, trace, warn};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinSet;

/// Concurrently verifies a list of pads by attempting to fetch data from their addresses.
///
/// Pads that are successfully fetched are considered verified. Pads that result in a
/// "RecordNotFound" error or other network errors are considered failed/discarded.
///
/// Returns a tuple containing a list of verified pads (address and key bytes) and the count of failed pads.
pub(crate) async fn verify_pads_concurrently(
    network_adapter: Arc<dyn NetworkAdapter>, // Use Arc for sharing across tasks
    pads_to_verify: Vec<(ScratchpadAddress, Vec<u8>)>,
    mut callback: Option<PurgeCallback>,
) -> Result<(Vec<(ScratchpadAddress, Vec<u8>)>, usize), PadLifecycleError> {
    let initial_count = pads_to_verify.len();
    info!(
        "Verification: Starting concurrent check for {} pads.",
        initial_count
    );

    if initial_count == 0 {
        debug!("Verification: No pads to verify.");
        // Emit starting/complete events even if count is 0? Yes, for consistency.
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
        return Ok((Vec::new(), 0));
    }

    let verified_pads_arc = Arc::new(Mutex::new(Vec::with_capacity(initial_count)));
    let failed_pads_count_arc = Arc::new(AtomicUsize::new(0));
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
        let failed_clone = Arc::clone(&failed_pads_count_arc);

        join_set.spawn(async move {
            trace!("Verification task starting for address: {}", address);
            // Use get_raw as a simple existence check
            match adapter_clone.get_raw(&address).await {
                Ok(_) => {
                    // Pad exists on the network
                    debug!(
                        "Successfully verified pad existence at address: {}",
                        address
                    );
                    let mut verified_guard = verified_clone.lock().await;
                    verified_guard.push((address, key_bytes)); // Keep original key bytes
                    Ok(true) // Indicate success
                }
                Err(e) => {
                    // Check if it's a 'not found' error vs. other network issue
                    // Relying on string matching is fragile, but necessary without specific error variants
                    let error_string = e.to_string();
                    if error_string.contains("RecordNotFound")
                        || error_string.contains("Could not find record")
                    {
                        info!(
                            "Pad at address {} not found on network. Discarding.",
                            address
                        );
                    } else {
                        warn!(
                            "Network error verifying pad at address {}: {}. Discarding.",
                            address, e
                        );
                    }
                    failed_clone.fetch_add(1, Ordering::SeqCst);
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
                // If task_result was Ok(true), it means success, already handled by pushing to verified_pads_arc
            }
            Err(join_error) => {
                // Task panicked or was cancelled
                error!("JoinError during pad verification: {}", join_error);
                failed_pads_count_arc.fetch_add(1, Ordering::SeqCst); // Count join errors as failures
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

    let final_failed_count = failed_pads_count_arc.load(Ordering::SeqCst);
    // Use into_inner after Arc::try_unwrap
    let verified_pads = Arc::try_unwrap(verified_pads_arc)
        .map_err(|_| {
            PadLifecycleError::InternalError("Failed to unwrap verified pads Arc".to_string())
        })?
        .into_inner(); // Consumes the Mutex

    let final_verified_count = verified_pads.len();

    info!(
        "Verification complete. Verified: {}, Discarded/Failed: {}",
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
        Ok((verified_pads, final_failed_count))
    }
}
