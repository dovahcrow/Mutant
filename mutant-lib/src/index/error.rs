use thiserror::Error;
#[derive(Debug, Error)]
pub enum IndexError {
    #[error("Key already exists: {0}")]
    KeyAlreadyExists(String),

    #[error("Cannot remove public upload: {0}")]
    CannotRemovePublicUpload(String),
}
