use std::sync::Arc;
use tokio::sync::RwLock;
use colonylib::{KeyStore, DataStore, Graph, PodManager};
use serde_json::Value;
use autonomi::{Client, Wallet};
use mutant_lib::config::NetworkChoice;
use crate::error::Error as DaemonError;
use hex;
use sha2::{Digest, Sha256};
use autonomi::client::key_derivation::{DerivationIndex, MainSecretKey};
use autonomi::{SecretKey, PublicKey};

/// Default testnet private key used for development and testing
const DEV_TESTNET_PRIVATE_KEY_HEX: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

/// Colony integration manager for MutAnt daemon
///
/// This module provides indexing and search capabilities for public content
/// stored on the Autonomi network using ColonyLib's RDF-based metadata system.
pub struct ColonyManager {
    key_store: Arc<RwLock<KeyStore>>,
    data_store: Arc<RwLock<DataStore>>,
    graph: Arc<RwLock<Graph>>,
    wallet: Wallet,
    network_choice: NetworkChoice,
    initialized: bool,
    /// The user's pod public address derived from the main secret key
    pod_public_address: String,
}

fn index(i: u64) -> DerivationIndex {

    let mut bytes = [0u8; 32];

    bytes[..8].copy_from_slice(&i.to_ne_bytes());

    DerivationIndex::from_bytes(bytes)

}

impl ColonyManager {


    /// Initialize the colony manager with default configuration
    ///
    /// This will create or load existing colony data from the standard data directory.
    /// The manager can operate in public-only mode for search operations.
    pub async fn new(wallet: Wallet, network_choice: NetworkChoice, private_key_hex: Option<String>) -> Result<Self, DaemonError> {
        // Initialize data store (creates directories if needed)
        let data_store = DataStore::create()
            .map_err(|e| DaemonError::ColonyError(format!("Failed to create data store: {}", e)))?;
        
        // Initialize or load key store
        let key_store_path = data_store.get_keystore_path();
        let mut key_store = if key_store_path.exists() {
            log::info!("Loading existing colony key store");
            let mut file = std::fs::File::open(&key_store_path)
                .map_err(|e| DaemonError::ColonyError(format!("Failed to open key store: {}", e)))?;
            KeyStore::from_file(&mut file, "password")
                .map_err(|e| DaemonError::ColonyError(format!("Failed to load key store: {}", e)))?
        } else {
            log::info!("Creating new colony key store from environment mnemonic");
            // Get mnemonic from environment variable, fall back to default if not set
            let mnemonic = std::env::var("COLONY_MNEMONIC")
                .unwrap_or_else(|_| {
                    log::warn!("COLONY_MNEMONIC environment variable not set, using default mnemonic");
                    "distance unusual problem mail service tide talk term lonely weather cheap patrol".to_string()
                });
            KeyStore::from_mnemonic(&mnemonic)
                .map_err(|e| DaemonError::ColonyError(format!("Failed to create key store: {}", e)))?
        };

        // Set the wallet key using the provided private key or fall back to default
        let wallet_key = match private_key_hex {
            Some(key) => {
                log::info!("Using provided private key for colony wallet");
                // Ensure the key has 0x prefix
                if key.starts_with("0x") {
                    key
                } else {
                    format!("0x{}", key)
                }
            }
            None => {
                log::warn!("No private key provided for colony manager");
                return Err(DaemonError::ColonyError("Private key required for colony manager initialization".to_string()));
            }
        };

        key_store.set_wallet_key(wallet_key.clone())
            .map_err(|e| DaemonError::ColonyError(format!("Failed to set wallet key: {}", e)))?;

        // Derive the pod public address from the wallet key
        let pod_public_address = key_store.get_address_at_index(1).unwrap();
        log::info!("Derived pod public address: {}", pod_public_address);

        // Initialize graph database
        let graph_path = data_store.get_graph_path();
        let graph = Graph::open(&graph_path)
            .map_err(|e| DaemonError::ColonyError(format!("Failed to open graph database: {}", e)))?;

        Ok(ColonyManager {
            key_store: Arc::new(RwLock::new(key_store)),
            data_store: Arc::new(RwLock::new(data_store)),
            graph: Arc::new(RwLock::new(graph)),
            wallet,
            network_choice,
            initialized: true,
            pod_public_address,
        })
    }

    /// Get a client configured for the correct network
    async fn get_client(&self) -> Result<Client, DaemonError> {
        match self.network_choice {
            NetworkChoice::Mainnet => Client::init().await
                .map_err(|e| DaemonError::ColonyError(format!("Failed to initialize mainnet client: {}", e))),
            NetworkChoice::Devnet => Client::init_local().await
                .map_err(|e| DaemonError::ColonyError(format!("Failed to initialize devnet client: {}", e))),
            NetworkChoice::Alphanet => {
                // For alphanet, we need to use init_with_config with specific configuration
                // For now, fall back to mainnet init - this should be improved
                Client::init().await
                    .map_err(|e| DaemonError::ColonyError(format!("Failed to initialize alphanet client: {}", e)))
            }
        }
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
        let client = self.get_client().await?;

        // Get locks and hold them for the duration of the operation
        let mut data_store = self.data_store.write().await;
        let mut key_store = self.key_store.write().await;
        let mut graph = self.graph.write().await;

        let mut pod_manager = PodManager::new(
            client,
            &self.wallet,
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
    
    /// Auto-generate metadata for a public upload and add it to the user's pod
    ///
    /// This is called automatically when a public file is uploaded successfully.
    /// It generates appropriate Schema.org metadata based on the file information.
    pub async fn auto_index_public_upload(
        &self,
        user_key: &str,
        filename: Option<String>,
        file_size: u64,
        public_address: String,
    ) -> Result<(), DaemonError> {
        if !self.initialized {
            log::warn!("Colony manager not initialized, skipping auto-indexing for key: {}", user_key);
            return Ok(()); // Don't fail the upload if colony isn't available
        }

        log::info!("Auto-indexing public upload: key={}, filename={:?}, size={}, public_address={}",
                   user_key, filename, file_size, public_address);

        // Generate metadata based on file information
        let metadata = self.generate_file_metadata(user_key, filename.as_deref(), file_size, &public_address);

        // Use existing index_content method
        match self.index_content(user_key, metadata, Some(public_address)).await {
            Ok(_) => {
                log::info!("Successfully auto-indexed public upload for key: {}", user_key);
                Ok(())
            }
            Err(e) => {
                log::error!("Failed to auto-index public upload for key {}: {}", user_key, e);
                // Don't fail the upload if colony indexing fails
                Ok(())
            }
        }
    }

    /// Generate Schema.org metadata for a file upload
    fn generate_file_metadata(&self, user_key: &str, filename: Option<&str>, file_size: u64, public_address: &str) -> Value {
        let file_name = filename.unwrap_or(user_key);

        // Determine content type based on file extension
        let (schema_type, encoding_format) = self.determine_content_type(file_name);

        // Use proper Schema.org format with schema: prefix like the example
        serde_json::json!({
            "@context": {"schema": "http://schema.org/"},
            "@type": schema_type,
            "@id": format!("ant://{}", public_address),
            "schema:name": file_name,
            "schema:description": format!("File uploaded by {}", user_key),
            "schema:contentSize": file_size,
            "schema:encodingFormat": encoding_format,
            "schema:author": user_key,
            "schema:dateCreated": chrono::Utc::now().to_rfc3339(),
            "schema:url": public_address
        })
    }

    /// Determine Schema.org content type and encoding format based on filename
    fn determine_content_type(&self, filename: &str) -> (&'static str, &'static str) {
        let extension = std::path::Path::new(filename)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_lowercase();

        match extension.as_str() {
            // Video formats
            "mp4" => ("schema:VideoObject", "video/mp4"),
            "avi" => ("schema:VideoObject", "video/avi"),
            "mkv" => ("schema:VideoObject", "video/mkv"),
            "mov" => ("schema:VideoObject", "video/mov"),
            "wmv" => ("schema:VideoObject", "video/wmv"),
            "flv" => ("schema:VideoObject", "video/flv"),
            "webm" => ("schema:VideoObject", "video/webm"),

            // Audio formats
            "mp3" => ("schema:AudioObject", "audio/mp3"),
            "wav" => ("schema:AudioObject", "audio/wav"),
            "flac" => ("schema:AudioObject", "audio/flac"),
            "aac" => ("schema:AudioObject", "audio/aac"),
            "ogg" => ("schema:AudioObject", "audio/ogg"),
            "m4a" => ("schema:AudioObject", "audio/m4a"),

            // Image formats
            "jpg" | "jpeg" => ("schema:ImageObject", "image/jpeg"),
            "png" => ("schema:ImageObject", "image/png"),
            "gif" => ("schema:ImageObject", "image/gif"),
            "bmp" => ("schema:ImageObject", "image/bmp"),
            "svg" => ("schema:ImageObject", "image/svg+xml"),
            "webp" => ("schema:ImageObject", "image/webp"),

            // Document formats
            "pdf" => ("schema:DigitalDocument", "application/pdf"),
            "doc" | "docx" => ("schema:DigitalDocument", "application/msword"),
            "txt" => ("schema:TextDigitalDocument", "text/plain"),
            "html" | "htm" => ("schema:WebPage", "text/html"),
            "json" => ("schema:Dataset", "application/json"),
            "xml" => ("schema:Dataset", "application/xml"),
            "csv" => ("schema:Dataset", "text/csv"),

            // Archive formats
            "zip" => ("schema:DataDownload", "application/zip"),
            "rar" => ("schema:DataDownload", "application/rar"),
            "7z" => ("schema:DataDownload", "application/7z"),
            "tar" => ("schema:DataDownload", "application/tar"),
            "gz" => ("schema:DataDownload", "application/gzip"),

            // Default for unknown types
            _ => ("schema:MediaObject", "application/octet-stream")
        }
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
        let client = self.get_client().await?;

        // Get locks and hold them for the duration of the operation
        let mut data_store = self.data_store.write().await;
        let mut key_store = self.key_store.write().await;
        let mut graph = self.graph.write().await;

        let mut pod_manager = PodManager::new(
            client,
            &self.wallet,
            &mut *data_store,
            &mut *key_store,
            &mut *graph,
        ).await
        .map_err(|e| DaemonError::ColonyError(format!("Failed to create pod manager: {}", e)))?;

        // Get or create the user's main pod
        let user_pod_address = self.get_or_create_user_pod(&mut pod_manager, user_key).await?;

        // Add the content metadata to the user's pod
        let subject_id = public_address.unwrap_or_else(|| format!("file_{}", user_key.replace(".", "_")));
        let metadata_str = serde_json::to_string(&metadata)
            .map_err(|e| DaemonError::ColonyError(format!("Failed to serialize metadata: {}", e)))?;

        log::debug!("Generated metadata JSON: {}", metadata_str);
        log::debug!("Subject ID: {}", subject_id);
        log::debug!("User pod address: {}", user_pod_address);

        pod_manager.put_subject_data(&user_pod_address, &subject_id, &metadata_str).await
            .map_err(|e| DaemonError::ColonyError(format!("Failed to add metadata to pod (pod_address: {}, subject_id: {}, metadata_len: {}): {}",
                user_pod_address, subject_id, metadata_str.len(), e)))?;

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
        let (pod_address, _scratchpad_address) = pod_manager.add_pod("main").await
            .map_err(|e| DaemonError::ColonyError(format!("Failed to create pod: {}", e)))?;

        // Add basic metadata about the pod itself (simplified format)
        let pod_metadata = serde_json::json!({
            "type": "UserPod",
            "name": format!("Content Pod for {}", user_key),
            "description": format!("Metadata pod containing all public content for user {}", user_key),
            "owner": user_key,
            "created": chrono::Utc::now().to_rfc3339()
        });

        let pod_metadata_str = serde_json::to_string(&pod_metadata)
            .map_err(|e| DaemonError::ColonyError(format!("Failed to serialize pod metadata: {}", e)))?;

        let pod_subject_id = format!("pod_{}", pod_address);
        log::debug!("Adding pod metadata: pod_address={}, subject_id={}, metadata_len={}",
                   pod_address, pod_subject_id, pod_metadata_str.len());

        pod_manager.put_subject_data(&pod_address, &pod_subject_id, &pod_metadata_str).await
            .map_err(|e| DaemonError::ColonyError(format!("Failed to add pod metadata (pod_address: {}, subject_id: {}, metadata_len: {}): {}",
                pod_address, pod_subject_id, pod_metadata_str.len(), e)))?;

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
        let client = self.get_client().await?;

        // Get locks and hold them for the duration of the operation
        let mut data_store = self.data_store.write().await;
        let mut key_store = self.key_store.write().await;
        let mut graph = self.graph.write().await;

        let mut pod_manager = PodManager::new(
            client,
            &self.wallet,
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

    /// Get the user's pod public address
    pub fn get_pod_address(&self) -> &str {
        &self.pod_public_address
    }

    /// Add a contact (pod address) to sync with
    pub async fn add_contact(&self, pod_address: &str, contact_name: Option<String>) -> Result<(), DaemonError> {
        if !self.initialized {
            return Err(DaemonError::ColonyError("Colony manager not initialized".to_string()));
        }

        log::info!("Adding contact pod: {} (name: {:?})", pod_address, contact_name);

        // Initialize client for pod operations
        let client = self.get_client().await?;

        // Get locks and hold them for the duration of the operation
        let mut data_store = self.data_store.write().await;
        let mut key_store = self.key_store.write().await;
        let mut graph = self.graph.write().await;

        let mut pod_manager = PodManager::new(
            client,
            &self.wallet,
            &mut *data_store,
            &mut *key_store,
            &mut *graph,
        ).await
        .map_err(|e| DaemonError::ColonyError(format!("Failed to create pod manager: {}", e)))?;

        // Use our pod public address instead of "main"
        pod_manager.add_pod_ref(&self.pod_public_address, pod_address)
            .map_err(|e| DaemonError::ColonyError(format!("Failed to add pod reference: {}", e)))?;

        log::debug!("Added pod reference: {} to our pod: {} at depth 1", pod_address, self.pod_public_address);

        // Now download and sync the contact's pod using refresh_ref
        pod_manager.refresh_ref(10).await
            .map_err(|e| DaemonError::ColonyError(format!("Failed to sync contact pod: {}", e)))?;

        pod_manager.upload_all().await
            .map_err(|e| DaemonError::ColonyError(format!("Failed to upload pod: {}", e)))?;

        log::info!("Successfully added and synced contact pod: {}", pod_address);
        Ok(())
    }

    /// List all available content from synced pods
    pub async fn list_all_content(&self) -> Result<Value, DaemonError> {
        if !self.initialized {
            return Err(DaemonError::ColonyError("Colony manager not initialized".to_string()));
        }

        log::debug!("Listing all content from synced pods");

        // Use a broad text search to get all content, then filter in the parsing stage
        // Since ColonyLib doesn't support custom SPARQL queries, we'll use text search
        let search_query = serde_json::json!({
            "type": "text",
            "text": "",
            "limit": 1000
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
        let client = self.get_client().await?;

        // Get locks and hold them for the duration of the operation
        let mut data_store = self.data_store.write().await;
        let mut key_store = self.key_store.write().await;
        let mut graph = self.graph.write().await;

        let mut pod_manager = PodManager::new(
            client,
            &self.wallet,
            &mut *data_store,
            &mut *key_store,
            &mut *graph,
        ).await
        .map_err(|e| DaemonError::ColonyError(format!("Failed to create pod manager: {}", e)))?;

        // Refresh cache to get latest data from all known pods
        pod_manager.refresh_ref(10).await
            .map_err(|e| DaemonError::ColonyError(format!("Failed to refresh cache: {}", e)))?;

        // Get count of synced pods from key store
        let pod_count = pod_manager.key_store.get_pointers().len();

        log::info!("Successfully synced {} pods", pod_count);
        Ok(pod_count)
    }

    /// Get the user's own contact information that can be shared with friends
    pub async fn get_user_contact_info(&self) -> Result<(String, String, Option<String>), DaemonError> {
        if !self.initialized {
            return Err(DaemonError::ColonyError("Colony manager not initialized".to_string()));
        }

        log::debug!("Getting user contact information");

        // Use the pod public address as the primary contact method
        let pod_address = self.pod_public_address.clone();
        let contact_type = "pod".to_string();

        // For display name, use a truncated version of the pod address
        let display_name = if pod_address.len() > 16 {
            Some(format!("{}...{}", &pod_address[0..8], &pod_address[pod_address.len()-8..]))
        } else {
            Some(pod_address.clone())
        };

        log::debug!("User contact info: pod_address={}, type={}, display_name={:?}",
                   pod_address, contact_type, display_name);

        Ok((pod_address, contact_type, display_name))
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
