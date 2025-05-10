#[macro_use]
mod console_log;
pub mod error;
mod game_wrapper;
mod socket;

pub use game_wrapper::{game, game_mut, GameWrapper, GAME_WRAPPER};
pub use socket::Socket;
