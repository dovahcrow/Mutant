use crate::index::PadInfo;
use autonomi::SecretKey;
use tokio::time::Duration;

pub(crate) const CONFIRMATION_RETRY_DELAY: Duration = Duration::from_secs(1);
pub(crate) const CONFIRMATION_RETRY_LIMIT: u32 = 360;

#[derive(Clone)]
pub(crate) struct WriteTaskInput {
    pub(crate) pad_info: PadInfo,
    pub(crate) secret_key: SecretKey,
    pub(crate) chunk_data: Vec<u8>,
}
