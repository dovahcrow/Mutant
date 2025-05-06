use crate::network::Network;
use crate::network::client::Config as ClientConfig;
use std::sync::Arc;

pub struct WorkerPoolConfig<Task> {
    pub network: Arc<Network>,
    pub client_config: ClientConfig,
    pub task_processor: Task,
    pub enable_recycling: bool,
    pub total_items_hint: usize,
}
