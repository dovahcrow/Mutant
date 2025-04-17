use crate::api::{ReserveCallback, ReserveEvent};
use crate::events::PurgeCallback;
use crate::events::PutCallback;
use crate::index::structure::PadStatus;
use crate::index::IndexManager;
use crate::network::NetworkError;
use crate::network::{NetworkAdapter, NetworkChoice};
use crate::pad_lifecycle::cache::{read_cached_index, write_cached_index};
use crate::pad_lifecycle::error::PadLifecycleError;
use crate::pad_lifecycle::import;
use crate::pad_lifecycle::pool::acquire_free_pad;
use crate::pad_lifecycle::verification::verify_pads_concurrently;
use crate::pad_lifecycle::PadOrigin;
use crate::storage::StorageManager;
use async_trait::async_trait;
use autonomi::{ScratchpadAddress, SecretKey};
use log::error;
use log::{debug, info, trace, warn};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::task::JoinSet;
use tokio::time::{interval, Duration};

#[async_trait]
pub trait PadLifecycleManager: Send + Sync {
    async fn initialize_index(
        &self,
        master_index_address: &ScratchpadAddress,
        master_index_key: &SecretKey,
        network_choice: NetworkChoice,
    ) -> Result<bool, PadLifecycleError>;

    async fn save_index_cache(
        &self,
        network_choice: NetworkChoice,
    ) -> Result<(), PadLifecycleError>;

    async fn acquire_pads(
        &self,
        count: usize,
        _callback: &mut Option<PutCallback>,
    ) -> Result<Vec<(ScratchpadAddress, SecretKey, PadOrigin)>, PadLifecycleError>;

    async fn purge(
        &self,
        callback: Option<PurgeCallback>,
        network_choice: NetworkChoice,
    ) -> Result<(), PadLifecycleError>;

    async fn import_external_pad(&self, private_key_hex: &str) -> Result<(), PadLifecycleError>;

    async fn reserve_pads(
        &self,
        count: usize,
        callback: Option<ReserveCallback>,
    ) -> Result<usize, PadLifecycleError>;
}

pub struct DefaultPadLifecycleManager {
    index_manager: Arc<dyn IndexManager>,
    network_adapter: Arc<dyn NetworkAdapter>,
    storage_manager: Arc<dyn StorageManager>,
}

impl DefaultPadLifecycleManager {
    pub fn new(
        index_manager: Arc<dyn IndexManager>,
        network_adapter: Arc<dyn NetworkAdapter>,
        storage_manager: Arc<dyn StorageManager>,
    ) -> Self {
        Self {
            index_manager,
            network_adapter,
            storage_manager,
        }
    }

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

    async fn purge(
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

    async fn import_external_pad(&self, private_key_hex: &str) -> Result<(), PadLifecycleError> {
        info!("PadLifecycleManager: Importing external pad...");
        import::import_pad(self.index_manager.as_ref(), private_key_hex).await
    }

    async fn reserve_pads(
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
            let storage_manager = Arc::clone(&self.storage_manager);

            join_set.spawn(async move {
                trace!("Reserve task {}: Generating key and address", i);
                let secret_key = SecretKey::random(); 
                let address = ScratchpadAddress::new(secret_key.public_key());
                trace!("Reserve task {}: Reserving {} on network via write_pad_data", i, address);

                
                match storage_manager
                    .write_pad_data(&secret_key, &[0u8], &PadStatus::Generated)
                    .await
                {
                    Ok(created_address) => {
                         if created_address != address {
                             warn!("write_pad_data returned address {} but expected {}", created_address, address);
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
                        error!("Reserve task {}: Failed network reservation (write_pad_data) for {}: {}", i, address, e);
                         
                         
                         
                         
                         Err(PadLifecycleError::Network(NetworkError::InternalError(format!(
                             "write_pad_data failed for {}: {}",
                             address, e
                         ))))
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

                                     invoke(ReserveEvent::PadReserved { address: reserved_address.clone() }, &maybe_callback).await?;
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
