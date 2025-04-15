use crate::events::PurgeCallback;
use crate::index::{IndexManager, PadInfo};
use crate::network::{NetworkAdapter, NetworkChoice};
use crate::pad_lifecycle::cache::{read_cached_index, write_cached_index};
use crate::pad_lifecycle::error::PadLifecycleError;
use crate::pad_lifecycle::import;
use crate::pad_lifecycle::pool::{acquire_free_pad, release_pads_to_free};
use crate::pad_lifecycle::verification::verify_pads_concurrently;
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

    /// Acquires a specified number of free pads from the pool.
    async fn acquire_pads(
        &self,
        count: usize,
    ) -> Result<Vec<(ScratchpadAddress, SecretKey)>, PadLifecycleError>;

    /// Releases a list of pads back to the free pool. Requires providing the key bytes.
    async fn release_pads(
        &self,
        pads_info: Vec<PadInfo>,
        pad_keys: &HashMap<ScratchpadAddress, Vec<u8>>, // Address -> Key Bytes map
    ) -> Result<(), PadLifecycleError>;

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
    ) -> Result<Vec<(ScratchpadAddress, SecretKey)>, PadLifecycleError> {
        info!("PadLifecycleManager: Acquiring {} pads...", count);
        let mut acquired_pads = Vec::with_capacity(count);
        for _ in 0..count {
            let pad = acquire_free_pad(self.index_manager.as_ref()).await?;
            acquired_pads.push(pad);
        }
        debug!("Successfully acquired {} pads.", acquired_pads.len());
        Ok(acquired_pads)
        // Note: This doesn't automatically save the index state change. Assumed handled later.
    }

    async fn release_pads(
        &self,
        pads_info: Vec<PadInfo>,
        pad_keys: &HashMap<ScratchpadAddress, Vec<u8>>,
    ) -> Result<(), PadLifecycleError> {
        info!("PadLifecycleManager: Releasing {} pads...", pads_info.len());
        release_pads_to_free(self.index_manager.as_ref(), pads_info, pad_keys).await
        // Note: This doesn't automatically save the index state change. Assumed handled later.
    }

    async fn purge(
        &self,
        callback: Option<PurgeCallback>,
        network_choice: NetworkChoice,
    ) -> Result<(), PadLifecycleError> {
        info!("PadLifecycleManager: Starting purge operation...");
        // 1. Take all pending pads from index
        let pending_pads = self.index_manager.take_pending_pads().await?;
        let initial_count = pending_pads.len();
        debug!("Took {} pads from pending list.", initial_count);

        // 2. Verify them concurrently
        let (verified_pads, failed_count) = verify_pads_concurrently(
            Arc::clone(&self.network_adapter), // Clone Arc for verification fn
            pending_pads,
            callback,
        )
        .await?;
        debug!(
            "Verification result: {} verified, {} failed.",
            verified_pads.len(),
            failed_count
        );

        // 3. Update index lists (add verified to free)
        self.index_manager
            .update_pad_lists(verified_pads, failed_count)
            .await?;
        debug!("Index lists updated with verification results.");

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
