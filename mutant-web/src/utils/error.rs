#[derive(Debug)]
pub enum Error {
    Core(ogame_core::error::Error),
    Socket(futures::channel::mpsc::SendError),
    InvalidCredentials,
    NetworkError(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<ogame_core::error::Error> for Error {
    fn from(e: ogame_core::error::Error) -> Self {
        Self::Core(e)
    }
}
impl From<futures::channel::mpsc::SendError> for Error {
    fn from(e: futures::channel::mpsc::SendError) -> Self {
        Self::Socket(e)
    }
}
