use std::sync::{Arc, MappedRwLockWriteGuard, RwLock, RwLockWriteGuard};
use std::sync::{MappedRwLockReadGuard, RwLockReadGuard};

use futures::{channel::mpsc::Sender, SinkExt};
use lazy_static::lazy_static;

use ogame_core::{game::Game, protocol::Protocol};

use crate::{app::notifications, utils::error::*};

lazy_static! {
    pub static ref GAME_WRAPPER: Arc<RwLock<Option<GameWrapper>>> = Arc::new(RwLock::new(None));
}

pub fn game<'a>() -> MappedRwLockReadGuard<'static, GameWrapper> {
    let guard = GAME_WRAPPER.read().unwrap();

    RwLockReadGuard::map(guard, |game_opt| match game_opt {
        Some(ref x) => x,
        None => panic!("Game not initialized"),
    })
}

pub fn game_mut<'a>() -> MappedRwLockWriteGuard<'static, GameWrapper> {
    let guard = GAME_WRAPPER.write().unwrap();

    RwLockWriteGuard::map(guard, |game_opt| match game_opt {
        Some(ref mut x) => x,
        None => panic!("Game not initialized"),
    })
}

#[derive(Clone)]
pub struct GameWrapper {
    pub game: Game,
    socket_sender: Sender<Protocol>,
}

impl PartialEq for GameWrapper {
    fn eq(&self, other: &Self) -> bool {
        self.game == other.game
    }
}

impl GameWrapper {
    pub fn new(game: Game, socket_sender: Sender<Protocol>) -> Self {
        Self {
            game,
            socket_sender,
        }
    }

    pub fn action(&mut self, message: Protocol) -> Result<()> {
        if let Err(e) = self.game.process_message(message.clone()) {
            notifications::error(format!("Error processing message: {:?}", e));
            return Err(e.into());
        }

        let mut socket_sender = self.socket_sender.clone();

        wasm_bindgen_futures::spawn_local(async move {
            socket_sender.send(message).await.unwrap();
        });

        Ok(())
    }
}

impl std::ops::Deref for GameWrapper {
    type Target = Game;

    fn deref(&self) -> &Self::Target {
        &self.game
    }
}

impl std::ops::DerefMut for GameWrapper {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.game
    }
}
