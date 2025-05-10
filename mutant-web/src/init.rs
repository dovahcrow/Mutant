use std::{
    cmp::min,
    sync::{Arc, RwLock},
};

use dotenvy_macro::dotenv;
use futures::{channel::mpsc::Receiver, StreamExt};
use log::{error, info};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use ogame_core::{
    game::{Faction, Game},
    market::{OrderSide, Trade},
    protocol::Protocol,
    GAME_DATA,
};
use wasm_bindgen::{prelude::Closure, JsCast};

use crate::{
    app::{notifications, ResearchWindow},
    game, game_mut,
    map::LoadingInfo,
    utils::{error::*, GameWrapper, Socket, GAME_WRAPPER},
};

fn start_game_tick() {
    let closure = Closure::wrap(Box::new(move || {
        let research = {
            let mut game = game_mut();

            let research = game.research.clone();

            // TODO: handle errors
            if let Err(e) = game.tick() {
                crate::app::notifications::error(format!("{:?}", e));
            }

            research
        };

        if research != game().research {
            ResearchWindow::regen_graph();
        }
    }) as Box<dyn FnMut()>);

    web_sys::window()
        .unwrap()
        .set_interval_with_callback_and_timeout_and_arguments_0(
            closure.as_ref().unchecked_ref(),
            1000,
        )
        .unwrap();

    closure.forget();
}

pub async fn game_init(loading_info: Arc<RwLock<LoadingInfo>>) -> Receiver<bool> {
    let (tx, rx) = futures::channel::mpsc::channel(1);
    let (ready_tx, ready_rx) = futures::channel::mpsc::channel(1);

    *GAME_WRAPPER.write().unwrap() = Some(GameWrapper::new(Game::new(), tx.clone()));

    start_game_tick();

    get_game_data(loading_info).await.unwrap();

    wasm_bindgen_futures::spawn_local(connect_socket(rx, ready_tx));

    // start_bots();

    ready_rx
}

#[allow(unused)]
pub fn start_bots() {
    let closure = Closure::wrap(Box::new(move || {
        let recipes = &GAME_DATA.read().recipes;
        for (_, recipe) in recipes {
            let rng = rand::thread_rng().gen_range(0..=1);
            let side = if rng == 0 { "buy" } else { "sell" }.to_string();

            let last_trade = game()
                .trades
                .values()
                .filter(|t| t.item_id == *recipe.name)
                .last()
                .unwrap_or(&Trade {
                    item_id: recipe.name.clone(),
                    price: 100,
                    volume: 100,
                    timestamp: 0,
                    id: "".to_string(),
                    market_id: "".to_string(),
                    buyer_id: "".to_string(),
                    seller_id: "".to_string(),
                })
                .clone();

            let price_rng = rand::thread_rng().gen_range(-10..=10);
            let price = last_trade.price as i64 + price_rng;

            let mut game = game_mut();
            game.action(Protocol::NewOrder {
                market_id: "".to_string(),
                item_id: recipe.name.clone(),
                side: OrderSide::from(side),
                price: price as usize,
                volume: 100,
            })
            .unwrap();
        }
    }) as Box<dyn FnMut()>);

    web_sys::window()
        .unwrap()
        .set_interval_with_callback_and_timeout_and_arguments_0(
            closure.as_ref().unchecked_ref(),
            100,
        )
        .unwrap();

    closure.forget();
}

pub async fn get_game_data(loading_info: Arc<RwLock<LoadingInfo>>) -> Result<()> {
    let client = reqwest::Client::new();

    let addr = dotenv!("ADDRESS");

    let response = client
        .get(format!("http://{}/public/game_data.cbor", addr))
        .send()
        .await
        .map_err(|e| {
            error!("error: {:?}", e);
        })
        .unwrap();

    let total_size = response.content_length().unwrap();

    // let response = response.bytes().await.unwrap();

    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();

    // a bytes buffer to store the downloaded data
    let mut buffer = Vec::new();

    loading_info.write().unwrap().message = "Downloading game data...".to_string();

    while let Some(item) = stream.next().await {
        let chunk = item.unwrap();
        buffer.extend_from_slice(&chunk);
        let new = min(downloaded + (chunk.len() as u64), total_size);
        downloaded = new;
        loading_info.write().unwrap().progress = (downloaded as f64 / total_size as f64) as f32;
    }

    loading_info.write().unwrap().message = "Parsing game data...".to_string();
    loading_info.write().unwrap().progress = 0.0;
    let game_data: ogame_core::GameData = serde_cbor::from_slice(&buffer[..]).unwrap();
    *ogame_core::GAME_DATA.write() = game_data;
    loading_info.write().unwrap().progress = 1.0;
    loading_info.write().unwrap().message = "Loading scene...".to_string();

    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Credentials {
    pub email: String,
    pub username: String,
    pub password: String,
    pub faction: Faction,
}

fn hash_password(password: &str) -> String {
    let salt = dotenv!("SALT");
    let salted = format!("{}{}", password, salt);

    let mut hasher = Sha256::new();
    hasher.update(salted.as_bytes());
    let result = hasher.finalize();

    format!("{:x}", result)
}

pub async fn send_auth(
    method: String,
    email: String,
    username: String,
    password: String,
    faction: Faction,
) -> Result<()> {
    let hashed_password = hash_password(&password);

    let credentials = Credentials {
        email,
        username,
        password: hashed_password,
        faction,
    };

    let client = reqwest::Client::new();
    let addr = dotenv!("ADDRESS");
    let response = client
        .post(format!("http://{}/auth/{}", addr, method))
        .json(&credentials)
        .send()
        .await
        .map_err(|e| {
            error!("error: {:?}", e);
            Error::NetworkError(format!("Failed to send auth request: {}", e))
        })?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err(Error::InvalidCredentials)
    }
}

pub async fn connect_socket(
    mut game_rx: Receiver<Protocol>,
    mut _ready: futures::channel::mpsc::Sender<bool>,
) {
    let addr = dotenv!("ADDRESS");

    let addr = format!("ws://{}/ws", addr);
    let mut ws: Socket<Protocol> = Socket::connect(&addr).await;

    let mut recv = ws.take_receiver().unwrap();

    wasm_bindgen_futures::spawn_local(async move {
        while let Some(msg) = recv.next().await {
            if let Protocol::Game(new_game) = msg {
                info!("Game: {:#?}", new_game);
                if new_game.version != game().version {
                    panic!(
                        "Game version mismatch (NEW){} != {}",
                        new_game.version,
                        game().version
                    );
                }
                game_mut().game = new_game;

                // ready.send(true).await;
            } else {
                if let Err(e) = game_mut().process_message(msg) {
                    notifications::error(format!("{:?}", e));
                }
            }
        }
    });

    while let Some(msg) = game_rx.next().await {
        ws.send(msg).await.unwrap();
    }
}
