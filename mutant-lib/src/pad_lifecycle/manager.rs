use crate::events::invoke_put_callback;
use crate::events::PurgeCallback;
use crate::events::PutCallback;
use crate::events::PutEvent;
use crate::index::{IndexManager, PadInfo};
use crate::network::{NetworkAdapter, NetworkChoice};
use crate::pad_lifecycle::cache::{read_cached_index, write_cached_index};
use crate::pad_lifecycle::error::PadLifecycleError;
use crate::pad_lifecycle::import;
use crate::pad_lifecycle::pool::acquire_free_pad;
use crate::pad_lifecycle::verification::verify_pads_concurrently;
use crate::pad_lifecycle::PadOrigin;
use async_trait::async_trait;
use autonomi::{ScratchpadAddress, SecretKey};
use log::error;
use log::{debug, info, warn};
use std::collections::HashMap;
use std::sync::Arc;

/// Trait defining the interface for managing the lifecycle of scratchpads,
/// including acquisition, release, verification, import, and index caching.
#[async_trait]
pub trait PadLifecycleManager: Send + Sync {
    /// Initializes the index state, trying the local cache first, then loading from the
    /// IndexManager's persistence layer.
    /// Returns Ok(true) if the index was loaded successfully (from cache or remote)
    /// Returns Ok(false) if the index was not found remotely and a default was initialized (prompt needed).
    /// Returns Err if any other error occurs during loading.
    async fn initialize_index(
        &self,
        master_index_address: &ScratchpadAddress,
        master_index_key: &SecretKey,
        network_choice: NetworkChoice,
    ) -> Result<bool, PadLifecycleError>;

    /// Saves the current in-memory index state (obtained from IndexManager) to the local cache.
    async fn save_index_cache(
        &self,
        network_choice: NetworkChoice,
    ) -> Result<(), PadLifecycleError>;

    /// Acquires a specified number of pads, indicating their origin and reporting progress.
    async fn acquire_pads(
        &self,
        count: usize,
        callback: &mut Option<PutCallback>,
    ) -> Result<Vec<(ScratchpadAddress, SecretKey, PadOrigin)>, PadLifecycleError>;

    /// Verifies pads in the pending list, updates the index, and saves the result ONLY to the local cache.
    async fn purge(
        &self,
        callback: Option<PurgeCallback>,
        network_choice: NetworkChoice,
    ) -> Result<(), PadLifecycleError>;

    /// Imports an external pad using its private key hex.
    async fn import_external_pad(&self, private_key_hex: &str) -> Result<(), PadLifecycleError>;
}

// --- Implementation ---

pub struct DefaultPadLifecycleManager {
    index_manager: Arc<dyn IndexManager>,
    network_adapter: Arc<dyn NetworkAdapter>,
}

impl DefaultPadLifecycleManager {
    pub fn new(
        index_manager: Arc<dyn IndexManager>,
        network_adapter: Arc<dyn NetworkAdapter>,
    ) -> Self {
        Self {
            index_manager,
            network_adapter,
        }
    }

    async fn generate_and_add_new_pad(
        &self,
    ) -> Result<(ScratchpadAddress, SecretKey), PadLifecycleError> {
        debug!("Generating a new pad...");
        // Generate new keypair
        let secret_key = SecretKey::random();
        let public_key = secret_key.public_key();
        let address = ScratchpadAddress::new(public_key);
        // let key_bytes = secret_key.to_bytes().to_vec(); // Key bytes no longer needed here

        // Pad is no longer added to pending here. It will be added to KeyInfo by the caller (store_op)
        // with status Generated.
        // self.index_manager
        //     .add_pending_pads(vec![(address, key_bytes)])
        //     .await?;
        // debug!("Generated new pad {}, it will be associated with a key shortly.", address);
        debug!("Generated new pad {}. Returning address and key.", address);

        Ok((address, secret_key))
    }
}

#[async_trait]
impl PadLifecycleManager for DefaultPadLifecycleManager {
    async fn initialize_index(
        &self,
        master_index_address: &ScratchpadAddress,
        master_index_key: &SecretKey,
        network_choice: NetworkChoice,
    ) -> Result<bool, PadLifecycleError> {
        info!("PadLifecycleManager: Initializing index state...");
        // 1. Try reading from cache
        match read_cached_index(network_choice).await {
            Ok(Some(cached_index)) => {
                info!("Index successfully loaded from local cache.");
                // Update in-memory state with cached version
                self.index_manager.update_index(cached_index).await?;
                Ok(true) // Index loaded successfully
            }
            Ok(None) => {
                info!("Local cache miss. Attempting load via IndexManager...");
                // 2. Cache miss, try loading via IndexManager (which handles remote load/default init)
                match self
                    .index_manager
                    .load_or_initialize(master_index_address, master_index_key)
                    .await
                {
                    Ok(_) => {
                        // load_or_initialize succeeded. Now check if it actually loaded something or used default.
                        // We can infer this by checking if the index is still default (e.g., empty index map).
                        // A more robust way might be for load_or_initialize to return status.
                        // Let's check if the index map is empty.
                        let index_copy = self.index_manager.get_index_copy().await?;
                        if index_copy.index.is_empty()
                            && index_copy.free_pads.is_empty()
                            && index_copy.pending_verification_pads.is_empty()
                        {
                            info!("IndexManager initialized a default index (not found remotely).");
                            // Save this default state to cache immediately? Yes, seems reasonable.
                            self.save_index_cache(network_choice).await?;
                            Ok(false) // Indicate prompt needed for remote creation
                        } else {
                            info!(
                                "Index successfully loaded via IndexManager (likely from remote)."
                            );
                            // Save the newly loaded index to cache
                            self.save_index_cache(network_choice).await?;
                            Ok(true) // Index loaded successfully
                        }
                    }
                    Err(e) => {
                        error!("Failed to load or initialize index via IndexManager: {}", e);
                        Err(PadLifecycleError::Index(e))
                    }
                }
            }
            Err(e) => {
                error!("Failed to read local cache: {}", e);
                // Proceed to load via IndexManager even if cache read fails? Yes.
                info!("Cache read failed. Attempting load via IndexManager...");
                match self
                    .index_manager
                    .load_or_initialize(master_index_address, master_index_key)
                    .await
                {
                    Ok(_) => {
                        let index_copy = self.index_manager.get_index_copy().await?;
                        if index_copy.index.is_empty()
                            && index_copy.free_pads.is_empty()
                            && index_copy.pending_verification_pads.is_empty()
                        {
                            info!("IndexManager initialized a default index (not found remotely).");
                            // Attempt cache save despite earlier read error
                            if let Err(cache_err) = self.save_index_cache(network_choice).await {
                                warn!("Failed to save default index to cache after initial read error: {}", cache_err);
                            }
                            Ok(false)
                        } else {
                            info!(
                                "Index successfully loaded via IndexManager (likely from remote)."
                            );
                            if let Err(cache_err) = self.save_index_cache(network_choice).await {
                                warn!("Failed to save loaded index to cache after initial read error: {}", cache_err);
                            }
                            Ok(true)
                        }
                    }
                    Err(load_err) => {
                        error!("Failed to load or initialize index via IndexManager after cache read error: {}", load_err);
                        Err(PadLifecycleError::Index(load_err)) // Return the load error
                    }
                }
            }
        }
    }

    async fn save_index_cache(
        &self,
        network_choice: NetworkChoice,
    ) -> Result<(), PadLifecycleError> {
        debug!("PadLifecycleManager: Saving index to local cache...");
        let index_copy = self.index_manager.get_index_copy().await?;
        write_cached_index(&index_copy, network_choice).await
    }

    async fn acquire_pads(
        &self,
        count: usize,
        callback: &mut Option<PutCallback>,
    ) -> Result<Vec<(ScratchpadAddress, SecretKey, PadOrigin)>, PadLifecycleError> {
        info!("PadLifecycleManager: Acquiring {} pads...", count);
        let mut acquired_pads = Vec::with_capacity(count);
        for i in 0..count {
            let pad_result = match acquire_free_pad(self.index_manager.as_ref()).await {
                Ok(pad) => {
                    // Pad came from the free pool
                    Ok((pad.0, pad.1, PadOrigin::FromFreePool))
                }
                Err(PadLifecycleError::PadAcquisitionFailed(msg))
                    if msg == "No free pads available in the index" =>
                {
                    // No free pads, generate a new one
                    debug!(
                        "No free pads available, generating new pad ({} out of {} needed)...",
                        i + 1,
                        count
                    );
                    match self.generate_and_add_new_pad().await {
                        Ok(new_pad) => Ok((new_pad.0, new_pad.1, PadOrigin::Generated)),
                        Err(e) => Err(e), // Propagate generation error
                    }
                }
                Err(e) => {
                    // Other error from acquire_free_pad, propagate it
                    Err(e)
                }
            };

            match pad_result {
                Ok(acquired_pad_tuple) => {
                    acquired_pads.push(acquired_pad_tuple);
                    // Emit PadReserved event for this single pad
                    if !invoke_put_callback(callback, PutEvent::PadReserved { count: 1 })
                        .await
                        // Map the top-level Error to PadLifecycleError::InternalError
                        .map_err(|e| {
                            PadLifecycleError::InternalError(format!(
                                "Put callback failed during pad acquisition: {}",
                                e
                            ))
                        })?
                    {
                        warn!("Pad acquisition cancelled by callback.");
                        return Err(PadLifecycleError::OperationCancelled);
                    }
                }
                Err(e) => {
                    error!("Failed to acquire pad {} out of {}: {}", i + 1, count, e);
                    // Return the error, caller handles partial state (which is now stored in index)
                    return Err(e);
                }
            }
        }
        // Sanity check
        if acquired_pads.len() != count {
            warn!("Acquire pads mismatch: requested {}, got {}. This might indicate an unexpected issue.", count, acquired_pads.len());
            // This might happen if cancelled, but we return Err(OperationCancelled) above.
            // If it happens otherwise, it's an internal issue.
            return Err(PadLifecycleError::InternalError(
                "Acquired pad count mismatch".to_string(),
            ));
        }
        debug!("Successfully acquired {} pads.", acquired_pads.len());
        Ok(acquired_pads)
    }

    async fn purge(
        &self,
        callback: Option<PurgeCallback>,
        network_choice: NetworkChoice,
    ) -> Result<(), PadLifecycleError> {
        info!("PadLifecycleManager: Starting purge operation...");

        // 1. Take all pads from the pending list
        let pads_to_verify = self.index_manager.take_pending_pads().await?;
        let initial_count = pads_to_verify.len();

        if initial_count == 0 {
            info!("Purge: No pads in pending list to verify.");
            // Callback handled within verify_pads_concurrently
        } else {
            info!("Purge: Verifying {} pads...", initial_count);
        }

        // 2. Verify pads concurrently
        let (verified_pads, failed_addresses) = verify_pads_concurrently(
            Arc::clone(&self.network_adapter),
            pads_to_verify, // Pass the taken pads
            callback,
        )
        .await?;

        // 3. Update index manager: Add verified pads to the free list
        info!(
            "Purge complete. {} verified (added to free list), {} failed/not found (discarded).",
            verified_pads.len(),
            failed_addresses.len() // Use length of failed_addresses
        );
        if !verified_pads.is_empty() {
            self.index_manager.add_free_pads(verified_pads).await?;
            debug!("Index lists updated with verification results (added verified to free list).");
        }

        // Note: Failed pads are implicitly removed because we called take_pending_pads earlier.

        // 4. Save the updated index ONLY to local cache
        self.save_index_cache(network_choice).await?;
        info!("Purge complete. Updated index saved to local cache.");

        Ok(())
    }

    async fn import_external_pad(&self, private_key_hex: &str) -> Result<(), PadLifecycleError> {
        info!("PadLifecycleManager: Importing external pad...");
        import::import_pad(self.index_manager.as_ref(), private_key_hex).await
        // Note: This doesn't automatically save the index state change. Assumed handled later by caller if needed.
        // However, the original import *did* save. Let's add a save here for consistency.
        // This requires getting the master index address/key somehow. Add to trait? No.
        // Let's make the caller responsible for saving after import.
    }
}
