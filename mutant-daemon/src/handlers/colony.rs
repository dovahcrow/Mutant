use std::sync::Arc;
use tokio::sync::OnceCell;
use mutant_lib::config::NetworkChoice;

use crate::error::Error as DaemonError;
use crate::colony::ColonyManager;
use mutant_protocol::{
    ErrorResponse, Response, SearchRequest, SearchResponse, IndexContentRequest,
    IndexContentResponse, GetMetadataRequest, GetMetadataResponse, AddContactRequest,
    AddContactResponse, ListContentRequest, ListContentResponse, SyncContactsRequest,
    SyncContactsResponse
};

use super::common::UpdateSender;

// Global colony manager instance
static COLONY_MANAGER: OnceCell<Arc<ColonyManager>> = OnceCell::const_new();

/// Initialize the global colony manager
pub async fn init_colony_manager(wallet: autonomi::Wallet, network_choice: NetworkChoice) -> Result<(), DaemonError> {
    let manager = ColonyManager::new(wallet, network_choice).await?;
    COLONY_MANAGER.set(Arc::new(manager))
        .map_err(|_| DaemonError::ColonyError("Colony manager already initialized".to_string()))?;
    
    log::info!("Colony manager initialized successfully");
    Ok(())
}

/// Get the global colony manager instance
pub async fn get_colony_manager() -> Result<Arc<ColonyManager>, DaemonError> {
    COLONY_MANAGER.get()
        .ok_or_else(|| DaemonError::ColonyError("Colony manager not initialized".to_string()))
        .map(|manager| manager.clone())
}

/// Handle search requests
/// 
/// Executes SPARQL queries against the local RDF graph database to find content
/// based on various criteria (text, type, predicate, etc.).
pub async fn handle_search(
    req: SearchRequest,
    update_tx: UpdateSender,
    _original_request_str: &str,
) -> Result<(), DaemonError> {
    log::debug!("Handling search request: {:?}", req);
    
    let colony_manager = get_colony_manager().await?;
    
    match colony_manager.search(req.query).await {
        Ok(results) => {
            let response = Response::Search(SearchResponse { results });
            
            if let Err(e) = update_tx.send(response) {
                log::error!("Failed to send search response: {}", e);
                return Err(DaemonError::Internal(format!("Failed to send search response: {}", e)));
            }
            
            log::debug!("Search request completed successfully");
            Ok(())
        }
        Err(e) => {
            log::error!("Search request failed: {}", e);
            let error_response = Response::Error(ErrorResponse {
                error: format!("Search failed: {}", e),
                original_request: Some(_original_request_str.to_string()),
            });
            
            if let Err(send_err) = update_tx.send(error_response) {
                log::error!("Failed to send error response: {}", send_err);
            }
            
            Err(e)
        }
    }
}

/// Handle content indexing requests
/// 
/// Creates metadata pods for stored content, enabling search and discovery.
/// The metadata should be in JSON-LD format following Schema.org conventions.
pub async fn handle_index_content(
    req: IndexContentRequest,
    update_tx: UpdateSender,
    _original_request_str: &str,
) -> Result<(), DaemonError> {
    log::debug!("Handling index content request: key={}, public_address={:?}", 
               req.user_key, req.public_address);
    
    let colony_manager = get_colony_manager().await?;
    
    match colony_manager.index_content(&req.user_key, req.metadata, req.public_address).await {
        Ok((success, pod_address)) => {
            let response = Response::IndexContent(IndexContentResponse {
                success,
                pod_address,
            });
            
            if let Err(e) = update_tx.send(response) {
                log::error!("Failed to send index content response: {}", e);
                return Err(DaemonError::Internal(format!("Failed to send index content response: {}", e)));
            }
            
            log::debug!("Index content request completed successfully");
            Ok(())
        }
        Err(e) => {
            log::error!("Index content request failed: {}", e);
            let error_response = Response::Error(ErrorResponse {
                error: format!("Content indexing failed: {}", e),
                original_request: Some(_original_request_str.to_string()),
            });
            
            if let Err(send_err) = update_tx.send(error_response) {
                log::error!("Failed to send error response: {}", send_err);
            }
            
            Err(e)
        }
    }
}

/// Handle metadata retrieval requests
/// 
/// Retrieves RDF metadata for a specific Autonomi address from the local graph database.
pub async fn handle_get_metadata(
    req: GetMetadataRequest,
    update_tx: UpdateSender,
    _original_request_str: &str,
) -> Result<(), DaemonError> {
    log::debug!("Handling get metadata request: address={}", req.address);
    
    let colony_manager = get_colony_manager().await?;
    
    match colony_manager.get_metadata(&req.address).await {
        Ok(metadata) => {
            let response = Response::GetMetadata(GetMetadataResponse { metadata });
            
            if let Err(e) = update_tx.send(response) {
                log::error!("Failed to send get metadata response: {}", e);
                return Err(DaemonError::Internal(format!("Failed to send get metadata response: {}", e)));
            }
            
            log::debug!("Get metadata request completed successfully");
            Ok(())
        }
        Err(e) => {
            log::error!("Get metadata request failed: {}", e);
            let error_response = Response::Error(ErrorResponse {
                error: format!("Metadata retrieval failed: {}", e),
                original_request: Some(_original_request_str.to_string()),
            });
            
            if let Err(send_err) = update_tx.send(error_response) {
                log::error!("Failed to send error response: {}", send_err);
            }
            
            Err(e)
        }
    }
}

/// Handle add contact requests
///
/// Adds a pod address to the list of contacts to sync with.
pub async fn handle_add_contact(
    req: AddContactRequest,
    update_tx: UpdateSender,
    _original_request_str: &str,
) -> Result<(), DaemonError> {
    log::debug!("Handling add contact request: pod_address={}, name={:?}",
               req.pod_address, req.contact_name);

    let colony_manager = get_colony_manager().await?;

    match colony_manager.add_contact(&req.pod_address, req.contact_name).await {
        Ok(()) => {
            let response = Response::AddContact(AddContactResponse {
                success: true,
            });

            if let Err(e) = update_tx.send(response) {
                log::error!("Failed to send add contact response: {}", e);
                return Err(DaemonError::Internal(format!("Failed to send add contact response: {}", e)));
            }

            log::debug!("Add contact request completed successfully");
            Ok(())
        }
        Err(e) => {
            log::error!("Add contact request failed: {}", e);
            let error_response = Response::Error(ErrorResponse {
                error: format!("Add contact failed: {}", e),
                original_request: Some(_original_request_str.to_string()),
            });

            if let Err(send_err) = update_tx.send(error_response) {
                log::error!("Failed to send error response: {}", send_err);
            }

            Err(e)
        }
    }
}

/// Handle list content requests
///
/// Lists all available content from synced pods.
pub async fn handle_list_content(
    _req: ListContentRequest,
    update_tx: UpdateSender,
    _original_request_str: &str,
) -> Result<(), DaemonError> {
    log::debug!("Handling list content request");

    let colony_manager = get_colony_manager().await?;

    match colony_manager.list_all_content().await {
        Ok(content) => {
            let response = Response::ListContent(ListContentResponse { content });

            if let Err(e) = update_tx.send(response) {
                log::error!("Failed to send list content response: {}", e);
                return Err(DaemonError::Internal(format!("Failed to send list content response: {}", e)));
            }

            log::debug!("List content request completed successfully");
            Ok(())
        }
        Err(e) => {
            log::error!("List content request failed: {}", e);
            let error_response = Response::Error(ErrorResponse {
                error: format!("List content failed: {}", e),
                original_request: Some(_original_request_str.to_string()),
            });

            if let Err(send_err) = update_tx.send(error_response) {
                log::error!("Failed to send error response: {}", send_err);
            }

            Err(e)
        }
    }
}

/// Handle sync contacts requests
///
/// Syncs all contact pods to get the latest content.
pub async fn handle_sync_contacts(
    _req: SyncContactsRequest,
    update_tx: UpdateSender,
    _original_request_str: &str,
) -> Result<(), DaemonError> {
    log::debug!("Handling sync contacts request");

    let colony_manager = get_colony_manager().await?;

    match colony_manager.sync_all_contacts().await {
        Ok(synced_count) => {
            let response = Response::SyncContacts(SyncContactsResponse { synced_count });

            if let Err(e) = update_tx.send(response) {
                log::error!("Failed to send sync contacts response: {}", e);
                return Err(DaemonError::Internal(format!("Failed to send sync contacts response: {}", e)));
            }

            log::debug!("Sync contacts request completed successfully");
            Ok(())
        }
        Err(e) => {
            log::error!("Sync contacts request failed: {}", e);
            let error_response = Response::Error(ErrorResponse {
                error: format!("Sync contacts failed: {}", e),
                original_request: Some(_original_request_str.to_string()),
            });

            if let Err(send_err) = update_tx.send(error_response) {
                log::error!("Failed to send error response: {}", send_err);
            }

            Err(e)
        }
    }
}

/// Refresh the colony cache from the network
/// 
/// This is a utility function that can be called periodically to update
/// the local RDF graph database with new pods from the network.
pub async fn refresh_colony_cache() -> Result<(), DaemonError> {
    log::info!("Refreshing colony cache from network");
    
    let colony_manager = get_colony_manager().await?;
    colony_manager.refresh_cache().await?;
    
    log::info!("Colony cache refresh completed");
    Ok(())
}

/// Save the colony key store
/// 
/// This ensures that derived keys are persisted to disk.
pub async fn save_colony_state() -> Result<(), DaemonError> {
    log::debug!("Saving colony state");
    
    let colony_manager = get_colony_manager().await?;
    colony_manager.save_key_store().await?;
    
    log::debug!("Colony state saved successfully");
    Ok(())
}

/// Check if the colony manager is initialized
pub fn is_colony_initialized() -> bool {
    COLONY_MANAGER.get().is_some()
}

/// Shutdown the colony manager
/// 
/// This saves the current state and cleans up resources.
pub async fn shutdown_colony() -> Result<(), DaemonError> {
    if is_colony_initialized() {
        log::info!("Shutting down colony manager");
        save_colony_state().await?;
        log::info!("Colony manager shutdown completed");
    }
    Ok(())
}
