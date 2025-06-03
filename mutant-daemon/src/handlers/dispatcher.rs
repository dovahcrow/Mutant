use std::sync::Arc;

use crate::error::Error as DaemonError;
use super::{TaskMap, ActiveKeysMap};

use mutant_lib::MutAnt;
use mutant_protocol::Request;

use super::common::UpdateSender;
use super::data_operations::{handle_get, handle_put, handle_put_data, handle_rm, handle_mv};
use super::import_export::{handle_export, handle_import};
use super::metadata::{handle_list_keys, handle_stats};
use super::system_operations::{handle_health_check, handle_purge, handle_sync};
use super::task_management::{handle_list_tasks, handle_query_task, handle_stop_task};
use super::colony::{handle_search, handle_index_content, handle_get_metadata, handle_add_contact, handle_list_content, handle_sync_contacts, handle_get_user_contact};

pub(crate) async fn handle_request(
    request: Request,
    original_request_str: &str,
    update_tx: UpdateSender,
    mutant: Arc<MutAnt>,
    tasks: TaskMap,
    active_keys: ActiveKeysMap,
) -> Result<(), DaemonError> {
    match request {
        Request::Put(put_req) => handle_put(put_req, update_tx, mutant, tasks, active_keys, original_request_str).await?,
        Request::PutData(put_data_req) => handle_put_data(put_data_req, update_tx, tasks, mutant, active_keys, original_request_str).await?,
        Request::Get(get_req) => handle_get(get_req, update_tx, mutant, tasks, active_keys, original_request_str).await?,
        Request::QueryTask(query_req) => {
            handle_query_task(query_req, update_tx, tasks, original_request_str).await?
        }
        Request::ListTasks(list_req) => handle_list_tasks(list_req, update_tx, tasks).await?,
        Request::Rm(rm_req) => handle_rm(rm_req, update_tx, mutant, active_keys, original_request_str).await?,
        Request::Mv(mv_req) => handle_mv(mv_req, update_tx, mutant, active_keys, original_request_str).await?,
        Request::ListKeys(list_keys_req) => {
            handle_list_keys(list_keys_req, update_tx, mutant).await?
        }
        Request::Stats(stats_req) => handle_stats(stats_req, update_tx, mutant).await?,
        Request::Sync(sync_req) => handle_sync(sync_req, update_tx, mutant, tasks).await?,
        Request::Purge(purge_req) => handle_purge(purge_req, update_tx, mutant, tasks).await?,
        Request::Import(import_req) => handle_import(import_req, update_tx, mutant).await?,
        Request::Export(export_req) => handle_export(export_req, update_tx, mutant).await?,
        Request::HealthCheck(health_check_req) => {
            handle_health_check(health_check_req, update_tx, mutant, tasks).await?
        }
        Request::StopTask(stop_task_req) => {
            handle_stop_task(stop_task_req, update_tx, tasks).await?
        }
        // Colony integration requests
        Request::Search(search_req) => {
            handle_search(search_req, update_tx, original_request_str).await?
        }
        Request::IndexContent(index_req) => {
            handle_index_content(index_req, update_tx, original_request_str).await?
        }
        Request::GetMetadata(metadata_req) => {
            handle_get_metadata(metadata_req, update_tx, original_request_str).await?
        }
        Request::AddContact(add_contact_req) => {
            handle_add_contact(add_contact_req, update_tx, original_request_str).await?
        }
        Request::ListContent(list_content_req) => {
            handle_list_content(list_content_req, update_tx, original_request_str).await?
        }
        Request::SyncContacts(sync_contacts_req) => {
            handle_sync_contacts(sync_contacts_req, update_tx, original_request_str).await?
        }
        Request::GetUserContact(get_user_contact_req) => {
            handle_get_user_contact(get_user_contact_req, update_tx, original_request_str).await?
        }
    }
    Ok(())
}
