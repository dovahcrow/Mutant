use crate::error::Error as LibError;

use autonomi::ScratchpadAddress;
use std::future::Future;
use std::pin::Pin;

pub mod init;
pub mod mutant;

pub use mutant::MutAnt;

use crate::events::PutEvent;
pub type PutCallback = Box<
    dyn Fn(PutEvent) -> Pin<Box<dyn Future<Output = Result<bool, LibError>> + Send>> + Send + Sync,
>;

use crate::events::GetEvent;
pub type GetCallback = Box<
    dyn Fn(GetEvent) -> Pin<Box<dyn Future<Output = Result<bool, LibError>> + Send>> + Send + Sync,
>;

#[derive(Debug, Clone)]
pub enum ReserveEvent {
    Starting { total_requested: usize },
    PadReserved { address: ScratchpadAddress },
    SavingIndex { reserved_count: usize },
    Complete { succeeded: usize, failed: usize },
}

pub type ReserveCallback = Box<
    dyn Fn(ReserveEvent) -> Pin<Box<dyn Future<Output = Result<bool, LibError>> + Send>>
        + Send
        + Sync,
>;

pub use crate::events;
