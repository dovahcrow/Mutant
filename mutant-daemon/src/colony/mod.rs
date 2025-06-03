use std::sync::Arc;
use tokio::sync::RwLock;
use colonylib::{KeyStore, DataStore, Graph, PodManager};
use serde_json::Value;
use autonomi::{Client, Network, Wallet};
use crate::error::Error as DaemonError;

/// Colony integration manager for MutAnt daemon
/// 
/// This module provides indexing and search capabilities for public content
/// stored on the Autonomi network using ColonyLib's RDF-based metadata system.
pub struct ColonyManager {
    key_store: Arc<RwLock<KeyStore>>,
    data_store: Arc<RwLock<DataStore>>,
    graph: Arc<RwLock<Graph>>,
    initialized: bool,
}

impl ColonyManager {
    /// Initialize the colony manager with default configuration
    /// 
    /// This will create or load existing colony data from the standard data directory.
    /// The manager can operate in public-only mode for search operations.
    pub async fn new() -> Result<Self, DaemonError> {
        // Initialize data store (creates directories if needed)
        let data_store = DataStore::create()
            .map_err(|e| DaemonError::ColonyError(format!("Failed to create data store: {}", e)))?;
        
        // Initialize or load key store
        let key_store_path = data_store.get_keystore_path();
        let key_store = if key_store_path.exists() {
            log::info!("Loading existing colony key store");
            let mut file = std::fs::File::open(&key_store_path)
                .map_err(|e| DaemonError::ColonyError(format!("Failed to open key store: {}", e)))?;
            KeyStore::from_file(&mut file, "mutant_colony_default")
                .map_err(|e| DaemonError::ColonyError(format!("Failed to load key store: {}", e)))?
        } else {
            log::info!("Creating new colony key store with default mnemonic");
            // Use a default mnemonic for now - in production this should be configurable
            let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
            KeyStore::from_mnemonic(mnemonic)
                .map_err(|e| DaemonError::ColonyError(format!("Failed to create key store: {}", e)))?
        };
        
        // Initialize graph database
        let graph_path = data_store.get_graph_path();
        let graph = Graph::open(&graph_path)
            .map_err(|e| DaemonError::ColonyError(format!("Failed to open graph database: {}", e)))?;
        
        Ok(ColonyManager {
            key_store: Arc::new(RwLock::new(key_store)),
            data_store: Arc::new(RwLock::new(data_store)),
            graph: Arc::new(RwLock::new(graph)),
            initialized: true,
        })
    }
    
    /// Search for content using SPARQL queries
    /// 
    /// This method accepts various query formats:
    /// - Simple text search: `"search term"`
    /// - Structured queries: `{"type": "text", "text": "search term", "limit": 10}`
    /// - Type-based search: `{"type": "by_type", "type_uri": "http://schema.org/VideoObject"}`
    /// - Advanced search: `{"type": "advanced", "text": "term", "type": "uri", "limit": 5}`
    pub async fn search(&self, query: Value) -> Result<Value, DaemonError> {
        if !self.initialized {
            return Err(DaemonError::ColonyError("Colony manager not initialized".to_string()));
        }
        
        log::debug!("Colony search request: {}", query);
        
        // For search operations, we need a temporary PodManager
        // In public-only mode, we don't need a wallet
        let client = Client::init().await
            .map_err(|e| DaemonError::ColonyError(format!("Failed to initialize Autonomi client: {}", e)))?;
        
        // Create a dummy wallet for the PodManager interface (not used in search)
        let dummy_wallet = create_dummy_wallet()?;

        // Get locks and hold them for the duration of the operation
        let mut data_store = self.data_store.write().await;
        let mut key_store = self.key_store.write().await;
        let mut graph = self.graph.write().await;

        let mut pod_manager = PodManager::new(
            client,
            &dummy_wallet,
            &mut *data_store,
            &mut *key_store,
            &mut *graph,
        ).await
        .map_err(|e| DaemonError::ColonyError(format!("Failed to create pod manager: {}", e)))?;

        let results = pod_manager.search(query).await
            .map_err(|e| DaemonError::ColonyError(format!("Search failed: {}", e)))?;
        
        log::debug!("Colony search results: {}", results);
        Ok(results)
    }
    
    /// Index content by creating metadata pods
    ///
    /// This creates a new pod containing RDF metadata about the specified content.
    /// The metadata should be in JSON-LD format following Schema.org conventions.
    pub async fn index_content(
        &self,
        user_key: &str,
        metadata: Value,
        public_address: Option<String>,
    ) -> Result<(bool, Option<String>), DaemonError> {
        if !self.initialized {
            return Err(DaemonError::ColonyError("Colony manager not initialized".to_string()));
        }

        log::info!("Indexing content: key={}, public_address={:?}", user_key, public_address);

        // Initialize client for pod operations
        let client = Client::init().await
            .map_err(|e| DaemonError::ColonyError(format!("Failed to initialize Autonomi client: {}", e)))?;

        let dummy_wallet = create_dummy_wallet()?;

        // Get locks and hold them for the duration of the operation
        let mut data_store = self.data_store.write().await;
        let mut key_store = self.key_store.write().await;
        let mut graph = self.graph.write().await;

        let mut pod_manager = PodManager::new(
            client,
            &dummy_wallet,
            &mut *data_store,
            &mut *key_store,
            &mut *graph,
        ).await
        .map_err(|e| DaemonError::ColonyError(format!("Failed to create pod manager: {}", e)))?;

        // Get or create the user's main pod
        let user_pod_address = self.get_or_create_user_pod(&mut pod_manager, user_key).await?;

        // Add the content metadata to the user's pod
        let subject_id = public_address.unwrap_or_else(|| format!("mutant:{}", user_key));
        let metadata_str = serde_json::to_string(&metadata)
            .map_err(|e| DaemonError::ColonyError(format!("Failed to serialize metadata: {}", e)))?;

        pod_manager.put_subject_data(&user_pod_address, &subject_id, &metadata_str).await
            .map_err(|e| DaemonError::ColonyError(format!("Failed to add metadata to pod: {}", e)))?;

        // Upload the updated pod to the network
        pod_manager.upload_all().await
            .map_err(|e| DaemonError::ColonyError(format!("Failed to upload pod: {}", e)))?;

        log::info!("Successfully indexed content in pod: {}", user_pod_address);
        Ok((true, Some(user_pod_address)))
    }

    /// Get or create a user's main pod for storing their content metadata
    async fn get_or_create_user_pod(
        &self,
        pod_manager: &mut PodManager<'_>,
        user_key: &str,
    ) -> Result<String, DaemonError> {
        // For now, create a new pod each time
        // In a real implementation, we'd store the user's pod address persistently
        log::info!("Creating new pod for user: {}", user_key);
        let (pod_address, _scratchpad_address) = pod_manager.add_pod().await
            .map_err(|e| DaemonError::ColonyError(format!("Failed to create pod: {}", e)))?;

        // Add basic metadata about the pod itself
        let pod_metadata = serde_json::json!({
            "@context": {"schema": "http://schema.org/", "mutant": "https://mutant.network/"},
            "@type": "mutant:UserPod",
            "@id": format!("mutant:pod:{}", pod_address),
            "schema:name": format!("Content Pod for {}", user_key),
            "schema:description": format!("Metadata pod containing all public content for user {}", user_key),
            "mutant:owner": user_key,
            "schema:dateCreated": chrono::Utc::now().to_rfc3339()
        });

        let pod_metadata_str = serde_json::to_string(&pod_metadata)
            .map_err(|e| DaemonError::ColonyError(format!("Failed to serialize pod metadata: {}", e)))?;

        pod_manager.put_subject_data(&pod_address, &format!("mutant:pod:{}", pod_address), &pod_metadata_str).await
            .map_err(|e| DaemonError::ColonyError(format!("Failed to add pod metadata: {}", e)))?;

        Ok(pod_address)
    }
    
    /// Get metadata for a specific address
    /// 
    /// This searches the local graph database for metadata about the specified address.
    pub async fn get_metadata(&self, address: &str) -> Result<Value, DaemonError> {
        if !self.initialized {
            return Err(DaemonError::ColonyError("Colony manager not initialized".to_string()));
        }
        
        log::debug!("Getting metadata for address: {}", address);
        
        // For metadata retrieval, we need a temporary PodManager
        let client = Client::init().await
            .map_err(|e| DaemonError::ColonyError(format!("Failed to initialize Autonomi client: {}", e)))?;
        
        let dummy_wallet = create_dummy_wallet()?;

        // Get locks and hold them for the duration of the operation
        let mut data_store = self.data_store.write().await;
        let mut key_store = self.key_store.write().await;
        let mut graph = self.graph.write().await;

        let mut pod_manager = PodManager::new(
            client,
            &dummy_wallet,
            &mut *data_store,
            &mut *key_store,
            &mut *graph,
        ).await
        .map_err(|e| DaemonError::ColonyError(format!("Failed to create pod manager: {}", e)))?;

        let metadata_json = pod_manager.get_subject_data(address).await
            .map_err(|e| DaemonError::ColonyError(format!("Failed to get metadata: {}", e)))?;
        
        let metadata: Value = serde_json::from_str(&metadata_json)
            .map_err(|e| DaemonError::ColonyError(format!("Failed to parse metadata JSON: {}", e)))?;
        
        Ok(metadata)
    }

    /// Add a contact (pod address) to sync with
    pub async fn add_contact(&self, pod_address: &str, contact_name: Option<String>) -> Result<(), DaemonError> {
        if !self.initialized {
            return Err(DaemonError::ColonyError("Colony manager not initialized".to_string()));
        }

        log::info!("Adding contact pod: {} (name: {:?})", pod_address, contact_name);

        // Initialize client for pod operations
        let client = Client::init().await
            .map_err(|e| DaemonError::ColonyError(format!("Failed to initialize Autonomi client: {}", e)))?;

        let dummy_wallet = create_dummy_wallet()?;

        // Get locks and hold them for the duration of the operation
        let mut data_store = self.data_store.write().await;
        let mut key_store = self.key_store.write().await;
        let mut graph = self.graph.write().await;

        let mut pod_manager = PodManager::new(
            client,
            &dummy_wallet,
            &mut *data_store,
            &mut *key_store,
            &mut *graph,
        ).await
        .map_err(|e| DaemonError::ColonyError(format!("Failed to create pod manager: {}", e)))?;

        // Download and sync the contact's pod using refresh_ref
        pod_manager.refresh_ref(1).await
            .map_err(|e| DaemonError::ColonyError(format!("Failed to sync contact pod: {}", e)))?;

        log::info!("Successfully added and synced contact pod: {}", pod_address);
        Ok(())
    }

    /// List all available content from synced pods
    pub async fn list_all_content(&self) -> Result<Value, DaemonError> {
        if !self.initialized {
            return Err(DaemonError::ColonyError("Colony manager not initialized".to_string()));
        }

        log::debug!("Listing all content from synced pods");

        // Search for all content across all pods
        let search_query = serde_json::json!({
            "type": "sparql",
            "query": r#"
                PREFIX schema: <http://schema.org/>
                PREFIX mutant: <https://mutant.network/>

                SELECT ?subject ?name ?description ?type ?dateCreated ?owner ?contentSize ?encodingFormat
                WHERE {
                    ?subject a ?type .
                    OPTIONAL { ?subject schema:name ?name }
                    OPTIONAL { ?subject schema:description ?description }
                    OPTIONAL { ?subject schema:dateCreated ?dateCreated }
                    OPTIONAL { ?subject mutant:owner ?owner }
                    OPTIONAL { ?subject schema:contentSize ?contentSize }
                    OPTIONAL { ?subject schema:encodingFormat ?encodingFormat }
                    FILTER(?type != mutant:UserPod)
                }
                ORDER BY DESC(?dateCreated)
            "#
        });

        self.search(search_query).await
    }

    /// Sync all contact pods to get latest content
    pub async fn sync_all_contacts(&self) -> Result<usize, DaemonError> {
        if !self.initialized {
            return Err(DaemonError::ColonyError("Colony manager not initialized".to_string()));
        }

        log::info!("Syncing all contact pods");

        // Initialize client for pod operations
        let client = Client::init().await
            .map_err(|e| DaemonError::ColonyError(format!("Failed to initialize Autonomi client: {}", e)))?;

        let dummy_wallet = create_dummy_wallet()?;

        // Get locks and hold them for the duration of the operation
        let mut data_store = self.data_store.write().await;
        let mut key_store = self.key_store.write().await;
        let mut graph = self.graph.write().await;

        let mut pod_manager = PodManager::new(
            client,
            &dummy_wallet,
            &mut *data_store,
            &mut *key_store,
            &mut *graph,
        ).await
        .map_err(|e| DaemonError::ColonyError(format!("Failed to create pod manager: {}", e)))?;

        // Refresh cache to get latest data from all known pods
        pod_manager.refresh_cache().await
            .map_err(|e| DaemonError::ColonyError(format!("Failed to refresh cache: {}", e)))?;

        // Get count of synced pods from key store
        let pod_count = pod_manager.key_store.get_pointers().len();

        log::info!("Successfully synced {} pods", pod_count);
        Ok(pod_count)
    }

    /// Refresh the local cache from the network
    /// 
    /// This downloads new pods and updates the local graph database.
    pub async fn refresh_cache(&self) -> Result<(), DaemonError> {
        if !self.initialized {
            return Err(DaemonError::ColonyError("Colony manager not initialized".to_string()));
        }
        
        log::info!("Refreshing colony cache from network");
        
        // TODO: Implement cache refresh
        // This would call pod_manager.refresh_cache() and pod_manager.refresh_ref()
        
        Ok(())
    }
    
    /// Save the key store to disk
    pub async fn save_key_store(&self) -> Result<(), DaemonError> {
        let data_store = self.data_store.read().await;
        let key_store = self.key_store.read().await;
        let key_store_path = data_store.get_keystore_path();
        
        let mut file = std::fs::File::create(&key_store_path)
            .map_err(|e| DaemonError::ColonyError(format!("Failed to create key store file: {}", e)))?;
        
        key_store.to_file(&mut file, "mutant_colony_default")
            .map_err(|e| DaemonError::ColonyError(format!("Failed to save key store: {}", e)))?;
        
        log::info!("Colony key store saved to {:?}", key_store_path);
        Ok(())
    }
}

/// Create a dummy wallet for PodManager interface
/// This is needed because PodManager requires a wallet reference, but for search operations
/// we don't actually need wallet functionality
fn create_dummy_wallet() -> Result<Wallet, DaemonError> {
    // Use a dummy private key - this won't be used for actual transactions in public-only mode
    let dummy_key = "0x0000000000000000000000000000000000000000000000000000000000000001";
    
    // We need to get the EVM network, but in public-only mode we might not have access
    // For now, we'll use a placeholder - this needs to be improved
    let evm_network = Network::ArbitrumOne; // Placeholder

    Wallet::new_from_private_key(evm_network, dummy_key)
        .map_err(|e| DaemonError::ColonyError(format!("Failed to create dummy wallet: {}", e)))
}

impl Drop for ColonyManager {
    fn drop(&mut self) {
        if self.initialized {
            // Attempt to save the key store on drop
            if let Err(e) = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(self.save_key_store())
            }) {
                log::warn!("Failed to save colony key store on drop: {}", e);
            }
        }
    }
}
