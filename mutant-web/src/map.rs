use std::collections::HashMap;

use crate::{
    app::{loading::LoadingWindow, login::LoginWindow, InfobarWindow},
    cam_test::PanCamPlugin,
    init::game_init,
};

use bevy::{asset::AssetMetaCheck, prelude::*};
use bevy_egui::EguiPlugin;
use egui::login;
use wasm_bindgen::JsCast;

mod bases;
mod cursor_world_position;
mod egui;
mod flights;
mod select_planet;
mod setup;
mod stationned_ships;
mod toggle_visibility;

#[derive(Component)]
struct System;

#[derive(Component)]
struct Planet(ogame_core::PlanetId);

#[derive(Component)]
struct Link;

#[derive(Component)]
struct FlightComponent(ogame_core::FlightId);

#[derive(Component)]
struct ShipComponent(ogame_core::ShipId);

#[derive(Component)]
struct BaseComponent(ogame_core::BaseId);

#[derive(Component)]
struct LabelComponent(LabelType);

#[derive(Component)]
struct Skybox;

#[derive(Resource, Default)]
struct CursorWorldPosition(Vec2);

#[derive(PartialEq, Eq)]
enum LabelType {
    System,
    Planet,
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Hash, States)]
pub enum AppState {
    #[default]
    Menu,
    LoadingData,
    LoadingScene,
    InGame,
}

#[derive(Clone, Default)]
pub struct LoadingInfo {
    pub message: String,
    pub progress: f32,
}

pub struct SessionCookie {}

impl SessionCookie {
    fn html_document() -> web_sys::HtmlDocument {
        let window = web_sys::window().unwrap();
        let document = window.document().unwrap();
        document.dyn_into::<web_sys::HtmlDocument>().unwrap()
    }

    fn get_cookies() -> HashMap<String, String> {
        let html_document = Self::html_document();
        let cookie = html_document.cookie().unwrap();
        let cookie = cookie.split(";").collect::<Vec<&str>>();
        cookie
            .iter()
            .filter_map(|cookie| {
                let cookie = cookie.trim();
                let cookie = cookie.split("=").collect::<Vec<&str>>();
                if cookie.len() != 2 {
                    return None;
                }
                Some((cookie[0].to_string(), cookie[1].to_string()))
            })
            .collect::<HashMap<String, String>>()
    }

    pub fn is_logged() -> bool {
        Self::get_cookies().get("access_token").is_some()
    }

    pub fn logout() {
        let html_document = Self::html_document();
        html_document
            .set_cookie("access_token=; expires=Thu, 01 Jan 1970 00:00:00 UTC")
            .unwrap();
    }
}

pub async fn start() {
    wasm_bindgen_futures::spawn_local(async move {
        crate::app::init().await;

        let window = web_sys::window().unwrap();

        let loading_info_res = LoadingWindow::default();
        let loading_info = loading_info_res.infos.clone();

        let state = if !SessionCookie::is_logged() {
            AppState::Menu
        } else {
            wasm_bindgen_futures::spawn_local(async move {
                game_init(loading_info).await;
            });
            AppState::LoadingData
        };

        App::new()
            .init_resource::<CursorWorldPosition>()
            .insert_resource(InfobarWindow::new())
            .insert_resource(loading_info_res)
            .add_plugins((
                DefaultPlugins
                    .set(WindowPlugin {
                        primary_window: Some(Window {
                            title: "I am a window!".into(),
                            name: Some("bevy.app".into()),
                            resolution: (
                                window.inner_width().unwrap().as_f64().unwrap() as f32,
                                window.inner_height().unwrap().as_f64().unwrap() as f32,
                            )
                                .into(),
                            canvas: Some("#canvas".to_string()),
                            // default window background color
                            ..default()
                        }),

                        ..default()
                    })
                    .set(AssetPlugin {
                        mode: AssetMode::Unprocessed,
                        file_path: "public".to_string(),
                        processed_file_path: "public".to_string(),
                        meta_check: AssetMetaCheck::Never,
                        ..default()
                    }),
                PanCamPlugin::default(),
            ))
            .insert_resource(ClearColor(Color::srgb(0., 0., 0.)))
            .add_plugins(EguiPlugin)
            .insert_state(state.clone())
            .init_resource::<LoginWindow>()
            .add_systems(
                Update,
                (
                    setup::scene::setup_scene,
                    setup::scene::spawn_systems,
                    setup::scene::spawn_links,
                    setup::scene::spawn_planets,
                )
                    // .chain()
                    .run_if(in_state(AppState::LoadingScene)),
            )
            .add_systems(Startup, setup::camera::setup_camera)
            .add_systems(Update, (login).run_if(in_state(AppState::Menu)))
            .add_systems(
                Update,
                (egui::loading).run_if(
                    in_state(AppState::LoadingData).or_else(in_state(AppState::LoadingScene)),
                ),
            )
            .add_systems(
                Update,
                (
                    update_skybox,
                    cursor_world_position::update_cursor_world_position,
                    select_planet::select_planet,
                    egui::egui_window_system,
                    flights::draw_flights,
                    stationned_ships::draw_stationned_ships,
                    bases::draw_bases,
                    toggle_visibility::update_visibility_with_scale,
                )
                    .run_if(in_state(AppState::InGame)),
            )
            .run();
    });
}

fn update_skybox(
    mut skybox: Query<&mut Transform, With<Skybox>>,
    camera: Query<(&OrthographicProjection, &Transform), (With<Camera>, Without<Skybox>)>,
) {
    let (projection, camera) = camera.single();

    for mut transform in skybox.iter_mut() {
        transform.translation = Vec3::new(camera.translation.x, camera.translation.y, 0.0);
        transform.scale = (projection.scale * 3., projection.scale * 3., 1.).into();
    }
}
