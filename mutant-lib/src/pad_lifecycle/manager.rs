use crate::api::{ReserveCallback, ReserveEvent};
use crate::internal_events::PurgeCallback;
use crate::internal_events::PutCallback;
use crate::index::structure::PadStatus;
use crate::index::manager::DefaultIndexManager;
use crate::network::{AutonomiNetworkAdapter, NetworkChoice};
use crate::pad_lifecycle::cache::{read_cached_index, write_cached_index};
use crate::pad_lifecycle::error::PadLifecycleError;
use crate::pad_lifecycle::import;
use crate::pad_lifecycle::pool::acquire_free_pad;
use crate::pad_lifecycle::verification::verify_pads_concurrently;
use crate::pad_lifecycle::PadOrigin;

use autonomi::{ScratchpadAddress, SecretKey};
use log::error;
use log::{debug, info, trace, warn};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::task::JoinSet;
use tokio::time::{interval, Duration};

/// Manages the lifecycle of scratchpads, including acquisition, verification, and caching.
///
/// Coordinates interactions between the `IndexManager`, `NetworkAdapter`, and local cache.
pub struct DefaultPadLifecycleManager {
    index_manager: Arc<DefaultIndexManager>,
    network_adapter: Arc<AutonomiNetworkAdapter>,
}

impl DefaultPadLifecycleManager {
    /// Creates a new `DefaultPadLifecycleManager`.
    ///
    /// # Arguments
    ///
    /// * `index_manager` - An `Arc` wrapped `DefaultIndexManager`.
    /// * `network_adapter` - An `Arc` wrapped `AutonomiNetworkAdapter`.
    pub fn new(
        index_manager: Arc<DefaultIndexManager>,
        network_adapter: Arc<AutonomiNetworkAdapter>,
    ) -> Self {
        Self {
            index_manager,
            network_adapter,
        }
    }

    /// Generates a new random secret key and corresponding scratchpad address.
    /// Does not interact with the index or network.
    ///
    /// # Errors
    ///
    /// Theoretically, random key generation could fail, but this is highly unlikely.
    /// (Currently does not return an error, but signature allows for it).
    async fn generate_and_add_new_pad(
        &self,
    ) -> Result<(ScratchpadAddress, SecretKey), PadLifecycleError> {
        debug!("Generating a new pad...");

        let secret_key = SecretKey::random();
        let public_key = secret_key.public_key();
        let address = ScratchpadAddress::new(public_key);

        debug!("Generated new pad {}. Returning address and key.", address);

        Ok((address, secret_key))
    }

    /// Initializes the index state, attempting to load from cache first, then network.
    ///
    /// If neither cache nor network index is found, it initializes a default empty index
    /// via the `IndexManager`.
    ///
    /// # Arguments
    ///
    /// * `master_index_address` - The address of the master index scratchpad on the network.
    /// * `master_index_key` - The secret key for the master index scratchpad.
    /// * `network_choice` - The network (Devnet or Mainnet) being used.
    ///
    /// # Errors
    ///
    /// Returns `PadLifecycleError` if:
    /// - Reading from cache fails (other than NotFound).
    /// - Loading or initializing via `IndexManager` fails (`PadLifecycleError::Index`).
    /// - Saving the loaded/initialized index to cache fails (`PadLifecycleError::CacheWriteError`).
    ///
    /// # Returns
    ///
    /// * `Ok(true)` if an index was successfully loaded from cache or network.
    /// * `Ok(false)` if a default index was initialized (no existing index found).
    pub async fn initialize_index(
        &self,
        master_index_address: &ScratchpadAddress,
        master_index_key: &SecretKey,
        network_choice: NetworkChoice,
    ) -> Result<bool, PadLifecycleError> {
        info!("PadLifecycleManager: Initializing index state...");

        match read_cached_index(network_choice).await {
            Ok(Some(cached_index)) => {
                info!("Index successfully loaded from local cache.");

                self.index_manager.update_index(cached_index).await?;
                Ok(true)
            }
            Ok(None) => {
                info!("Local cache miss. Attempting load via IndexManager...");

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

                            self.save_index_cache(network_choice).await?;
                            Ok(false)
                        } else {
                            info!(
                                "Index successfully loaded via IndexManager (likely from remote)."
                            );

                            self.save_index_cache(network_choice).await?;
                            Ok(true)
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
                        Err(PadLifecycleError::Index(load_err))
                    }
                }
            }
        }
    }

    /// Saves the current state of the `MasterIndex` (obtained from `IndexManager`) to the local cache.
    ///
    /// # Arguments
    ///
    /// * `network_choice` - The network (Devnet or Mainnet) to determine the cache file path.
    ///
    /// # Errors
    ///
    /// Returns `PadLifecycleError::CacheWriteError` if saving to the cache fails.
    /// Returns `PadLifecycleError::Index` if getting the index copy from the manager fails.
    pub async fn save_index_cache(
        &self,
        network_choice: NetworkChoice,
    ) -> Result<(), PadLifecycleError> {
        debug!("PadLifecycleManager: Saving index to local cache...");
        let index_copy = self.index_manager.get_index_copy().await?;
        write_cached_index(&index_copy, network_choice).await
    }

    /// Acquires a specified number of pads for use.
    ///
    /// It prioritizes taking pads from the `IndexManager`'s free list.
    /// If the free list is exhausted, it generates new pads.
    ///
    /// # Arguments
    ///
    /// * `count` - The number of pads to acquire.
    /// * `_callback` - (Currently unused) An optional callback for progress reporting.
    ///
    /// # Errors
    ///
    /// Returns `PadLifecycleError::PadAcquisitionFailed` if acquiring from the free pool
    /// or generating a new pad fails.
    /// Returns `PadLifecycleError::InternalError` if the final acquired count doesn't match
    /// the requested count.
    ///
    /// # Returns
    ///
    /// A vector of tuples, each containing the `ScratchpadAddress`, `SecretKey`,
    /// and `PadOrigin` for an acquired pad.
    pub async fn acquire_pads(
        &self,
        count: usize,
        _callback: &mut Option<PutCallback>,
    ) -> Result<Vec<(ScratchpadAddress, SecretKey, PadOrigin)>, PadLifecycleError> {
        info!("PadLifecycleManager: Acquiring {} pads...", count);
        let mut acquired_pads = Vec::with_capacity(count);
        for i in 0..count {
            let pad_result = match acquire_free_pad(self.index_manager.as_ref()).await {
                Ok(pad_tuple) => {
                    let (address, key, initial_counter) = pad_tuple;
                    Ok((address, key, PadOrigin::FreePool { initial_counter }))
                }
                Err(PadLifecycleError::PadAcquisitionFailed(msg))
                    if msg == "No free pads available in the index" =>
                {
                    debug!(
                        "No free pads available, generating new pad ({} out of {} needed)...",
                        i + 1,
                        count
                    );
                    match self.generate_and_add_new_pad().await {
                        Ok(new_pad) => Ok((new_pad.0, new_pad.1, PadOrigin::Generated)),
                        Err(e) => Err(e),
                    }
                }
                Err(e) => Err(e),
            };

            match pad_result {
                Ok(acquired_pad_tuple) => {
                    acquired_pads.push(acquired_pad_tuple);
                }
                Err(e) => {
                    error!("Failed to acquire pad {} out of {}: {}", i + 1, count, e);

                    return Err(e);
                }
            }
        }

        if acquired_pads.len() != count {
            warn!("Acquire pads mismatch: requested {}, got {}. This might indicate an unexpected issue.", count, acquired_pads.len());

            return Err(PadLifecycleError::InternalError(
                "Acquired pad count mismatch".to_string(),
            ));
        }
        debug!("Successfully acquired {} pads.", acquired_pads.len());

        Ok(acquired_pads)
    }

    /// Initiates the purge process: verifying pads marked as pending verification.
    ///
    /// Takes all pads from the `IndexManager`'s pending list, verifies their existence
    /// on the network concurrently, and updates the `IndexManager` lists (free or back to pending)
    /// based on the results. Finally, saves the updated index to the cache.
    ///
    /// # Arguments
    ///
    /// * `callback` - An optional callback function to report progress events during verification.
    /// * `network_choice` - The network (Devnet or Mainnet) to save the cache for.
    ///
    /// # Errors
    ///
    /// Returns `PadLifecycleError` if:
    /// - Taking pending pads from `IndexManager` fails (`PadLifecycleError::Index`).
    /// - Concurrent verification encounters errors (`PadLifecycleError::Network`, `PadLifecycleError::InternalError`).
    /// - Updating `IndexManager` lists fails (`PadLifecycleError::Index`).
    /// - Saving the index cache fails (`PadLifecycleError::CacheWriteError`).
    /// - A callback invocation fails or cancels the operation (`PadLifecycleError::InternalError`, `PadLifecycleError::OperationCancelled`).
    pub async fn purge(
        &self,
        callback: Option<PurgeCallback>,
        network_choice: NetworkChoice,
    ) -> Result<(), PadLifecycleError> {
        info!("PadLifecycleManager: Starting purge operation...");

        let pads_to_verify = self.index_manager.take_pending_pads().await?;
        let initial_count = pads_to_verify.len();

        if initial_count == 0 {
            info!("Purge: No pads in pending list to verify.");
        } else {
            info!("Purge: Verifying {} pads...", initial_count);
        }

        let (verified_pads, not_found_addresses, retry_pads) =
            verify_pads_concurrently(Arc::clone(&self.network_adapter), pads_to_verify, callback)
                .await?;

        info!(
            "Purge results: {} verified (added to free list), {} not found (discarded), {} errors (returned to pending list).",
            verified_pads.len(),
            not_found_addresses.len(), 
            retry_pads.len() 
        );
        if !verified_pads.is_empty() {
            self.index_manager.add_free_pads(verified_pads).await?;
            debug!("Index lists updated with verification results (added verified to free list).");
        }

        if !retry_pads.is_empty() {
            self.index_manager.add_pending_pads(retry_pads).await?;
            debug!("Index lists updated with verification results (added retry pads back to pending list).");
        }

        self.save_index_cache(network_choice).await?;
        info!("Purge complete. Updated index saved to local cache.");

        Ok(())
    }

    /// Imports an external pad using its private key hex string.
    ///
    /// Delegates to the `import::import_pad` function.
    ///
    /// # Arguments
    ///
    /// * `private_key_hex` - The hex-encoded private key of the pad to import.
    ///
    /// # Errors
    ///
    /// Returns `PadLifecycleError` if the import fails (e.g., invalid key, index error).
    pub async fn import_external_pad(&self, private_key_hex: &str) -> Result<(), PadLifecycleError> {
        info!("PadLifecycleManager: Importing external pad...");
        import::import_pad(self.index_manager.as_ref(), private_key_hex).await
    }

    /// Reserves a specified number of new pads on the network and adds them to the free list.
    ///
    /// This involves generating new keys, performing a minimal write (`put_raw` with `[0u8]`) 
    /// to reserve the address on the network, and then adding the pad to the `IndexManager`'s free list.
    /// The index cache is saved after each successful reservation.
    /// Operations are performed concurrently.
    ///
    /// # Arguments
    ///
    /// * `count` - The number of pads to reserve.
    /// * `callback` - An optional callback to report progress (Starting, PadReserved, SavingIndex, Complete).
    ///
    /// # Errors
    ///
    /// Returns `PadLifecycleError` if:
    /// - A network `put_raw` operation fails (`PadLifecycleError::Network`).
    /// - Adding the pad to the `IndexManager` fails (`PadLifecycleError::Index`).
    /// - Saving the index cache fails (`PadLifecycleError::CacheWriteError`).
    /// - A task join error occurs (`PadLifecycleError::InternalError`).
    /// - A callback invocation fails or cancels the operation (`PadLifecycleError::InternalError`, `PadLifecycleError::OperationCancelled`).
    ///
    /// # Returns
    ///
    /// The number of pads successfully reserved and added to the index.
    pub async fn reserve_pads(
        &self,
        count: usize,
        callback: Option<ReserveCallback>,
    ) -> Result<usize, PadLifecycleError> {
        debug!(
            "PadLifecycleManager: reserve_pads called for count={}",
            count
        );
        if count == 0 {
            info!("Reserve count is 0, nothing to do.");
            return Ok(0);
        }

        let network_choice = self.network_adapter.get_network_choice();
        let maybe_callback = callback.map(Arc::new);
        let invoke =
            |event: ReserveEvent,
             cb_arc: &Option<Arc<ReserveCallback>>|
             -> Pin<Box<dyn Future<Output = Result<bool, PadLifecycleError>> + Send>> {
                match cb_arc {
                    Some(cb) => {
                        let cb_clone = Arc::clone(cb);
                        Box::pin(async move {
                            match cb_clone(event).await {
                                Ok(true) => Ok(true),
                                Ok(false) => {
                                    warn!("Reserve operation cancelled by callback.");

                                    Err(PadLifecycleError::InternalError(
                                        "Cancelled by callback".to_string(),
                                    ))
                                }
                                Err(e) => Err(PadLifecycleError::InternalError(format!(
                                    "Callback failed: {}",
                                    e
                                ))),
                            }
                        })
                    }
                    None => Box::pin(async { Ok(true) }),
                }
            };

        invoke(
            ReserveEvent::Starting {
                total_requested: count,
            },
            &maybe_callback,
        )
        .await?;

        let mut join_set = JoinSet::new();
        let mut successful_creations = 0;
        let mut failed_creations = 0;

        for i in 0..count {
            let index_manager = Arc::clone(&self.index_manager);
            let network_adapter = Arc::clone(&self.network_adapter);

            join_set.spawn(async move {
                trace!("Reserve task {}: Generating key and address", i);
                let secret_key = SecretKey::random(); 
                let address = ScratchpadAddress::new(secret_key.public_key());
                trace!("Reserve task {}: Reserving {} on network via put_raw", i, address);

                
                match network_adapter
                    .put_raw(&secret_key, &[0u8], &PadStatus::Generated)
                    .await
                {
                    Ok(created_address) => {
                         if created_address != address {
                             warn!("put_raw returned address {} but expected {}", created_address, address);
                         }
                         trace!("Reserve task {}: Network reservation successful for {}", i, address);
                        
                        let key_bytes = secret_key.to_bytes().to_vec();
                        match index_manager
                            .add_free_pad(address, key_bytes) 
                            .await {
                                Ok(_) => {
                                    trace!("Reserve task {}: Added {} to index free list successfully", i, address);
                                    Ok(address) 
                                },
                                Err(e) => {
                                    error!(
                                        "Reserve task {}: Failed to add pad {} to index free list after network success: {}",
                                        i, address, e
                                    );
                                     
                                     Err(PadLifecycleError::Index(e))
                                }
                            }
                    },
                    Err(e) => {
                        error!("Reserve task {}: Failed network reservation (put_raw) for {}: {}", i, address, e);
                         
                         
                         
                         
                         Err(PadLifecycleError::Network(e))
                    }
                }
            });
        }

        let mut ticker = interval(Duration::from_millis(100));

        while !join_set.is_empty() {
            tokio::select! {
                 Some(result) = join_set.join_next() => {
                    match result {
                        Ok(Ok(reserved_address)) => {

                             trace!("Task succeeded for {}. Saving index cache...", reserved_address);
                             match self.save_index_cache(network_choice).await {
                                 Ok(_) => {
                                     successful_creations += 1;
                                     trace!("Index cache saved successfully after reserving {}", reserved_address);

                                     invoke(ReserveEvent::PadReserved { address: reserved_address }, &maybe_callback).await?;
                                     invoke(ReserveEvent::SavingIndex { reserved_count: successful_creations }, &maybe_callback).await?;
                                 }
                                 Err(e) => {
                                     error!("Failed to save index cache after reserving {}: {}. Counting as failure.", reserved_address, e);
                                     failed_creations += 1;
                                 }
                             }
                        }
                        Ok(Err(e)) => {
                            error!("A reservation task failed: {}", e);
                            failed_creations += 1;
                        }
                        Err(join_err) => {
                            error!("A reservation task join error occurred: {}", join_err);
                            failed_creations += 1;
                        }
                    }
                }
                 _ = ticker.tick(), if maybe_callback.is_some() => {}
            }

        }

        info!(
            "Pad reservation process complete: {} succeeded, {} failed",
            successful_creations, failed_creations
        );
        invoke(
            ReserveEvent::Complete {
                succeeded: successful_creations,
                failed: failed_creations,
            },
            &maybe_callback,
        )
        .await?;

        Ok(successful_creations)
    }
}
