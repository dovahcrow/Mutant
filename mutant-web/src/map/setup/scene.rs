use std::collections::{BTreeMap, BTreeSet};

use bevy::math::vec3;
use bevy::prelude::*;
use bevy::sprite::MaterialMesh2dBundle;
use bevy_egui::EguiContexts;
use ogame_core::{PlanetId, PositionedEntity, SystemId, GAME_DATA};
use rand::Rng;

use crate::{
    app::{focus_first_base, loading::LoadingWindow, window_system_mut, WindowSystem},
    map::{AppState, LabelComponent, LabelType, Link, Planet},
    utils::game,
};

pub const SYSTEM_RADIUS: f32 = 3500.;
pub const PLANET_RADIUS: f32 = 40.;

pub fn setup_scene(loading: ResMut<LoadingWindow>, _commands: Commands) {
    *window_system_mut() = WindowSystem::from_memory(game().user_id.clone());
    focus_first_base();

    loading.infos.write().unwrap().message = "Loading scene".to_string();
    loading.infos.write().unwrap().progress = 0.;

    /* commands.spawn((
        SpriteBundle {
            texture: asset_server.load("Starfield.png"),
            transform: Transform {
                translation: Vec3::new(0.0, 0.0, 10.0),
                scale: Vec3::new(2.0, 2.0, 1.0),
                ..default()
            },
            ..default()
        },
        Skybox,
    )); */
}

pub fn spawn_systems(
    mut commands: Commands,
    mut loading: ResMut<LoadingWindow>,
    mut egui_contexts: EguiContexts,
    state: ResMut<NextState<AppState>>,
) {
    let game_data = GAME_DATA.read();

    let text_style = TextStyle {
        font_size: 500.0,
        color: Color::srgba(1., 1., 1., 0.2),
        ..default()
    };

    loading.infos.write().unwrap().message = "Spawning systems".to_string();
    loading.infos.write().unwrap().progress = 0.;

    loading.render(egui_contexts.ctx_mut(), state);

    let mut nb_spawned = 0;

    for (_system_id, system) in &game_data.systems {
        commands.spawn((
            LabelComponent(LabelType::System),
            Text2dBundle {
                text: Text::from_section(system.name.clone(), text_style.clone())
                    .with_justify(JustifyText::Center),
                transform: Transform {
                    translation: vec3(system.x as f32, system.y as f32, 3.),
                    ..default()
                },
                ..default()
            },
        ));

        nb_spawned += 1;

        loading.infos.write().unwrap().progress =
            nb_spawned as f32 / game_data.systems.len() as f32;
    }
}

const SPRITES: [&str; 20] = [
    "large_planets/Solid/Airless/Airless_01-512x512.png",
    "large_planets/Solid/Aquamarine/Aquamarine_01-512x512.png",
    "large_planets/Solid/Arid/Arid_01-512x512.png",
    "large_planets/Solid/Barren/Barren_01-512x512.png",
    "large_planets/Solid/Cloudy/Cloudy_01-512x512.png",
    "large_planets/Solid/Cratered/Cratered_01-512x512.png",
    "large_planets/Solid/Dry/Dry_01-512x512.png",
    "large_planets/Solid/Frozen/Frozen_01-512x512.png",
    "large_planets/Solid/Glacial/Glacial_01-512x512.png",
    "large_planets/Solid/Icy/Icy_01-512x512.png",
    "large_planets/Solid/Lunar/Lunar_01-512x512.png",
    "large_planets/Solid/Lush/Lush_01-512x512.png",
    "large_planets/Solid/Magma/Magma_01-512x512.png",
    "large_planets/Solid/Muddy/Muddy_01-512x512.png",
    "large_planets/Solid/Oasis/Oasis_01-512x512.png",
    "large_planets/Solid/Ocean/Ocean_01-512x512.png",
    "large_planets/Solid/Rocky/Rocky_01-512x512.png",
    "large_planets/Solid/Snowy/Snowy_01-512x512.png",
    "large_planets/Solid/Terrestrial/Terrestrial_01-512x512.png",
    "large_planets/Solid/Tropical/Tropical_01-512x512.png",
];

pub fn spawn_planets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    asset_server: ResMut<AssetServer>,
    loading: ResMut<LoadingWindow>,
    mut state: ResMut<NextState<AppState>>,
) {
    let game_data = GAME_DATA.read();

    let text_style = TextStyle {
        font_size: 160.0,
        color: Color::srgba(1., 1., 1., 0.2),
        ..default()
    };

    let mut planets_by_systems: BTreeMap<SystemId, BTreeMap<PlanetId, ogame_core::Planet>> =
        BTreeMap::new();

    for (planet_id, planet) in &game_data.planets {
        planets_by_systems
            .entry(planet.system_id.clone())
            .or_insert_with(BTreeMap::new)
            .insert(planet_id.clone(), planet.clone());
    }

    let total_planets = planets_by_systems
        .iter()
        .map(|(_, planets)| planets.len())
        .sum::<usize>();

    let mut nb_spawned = 0;

    loading.infos.write().unwrap().message = "Spawning planets".to_string();
    loading.infos.write().unwrap().progress = 0.;

    for (_system_id, planets) in &planets_by_systems {
        let mut i = 5.;

        for (planet_id, planet) in planets {
            let (x, y) = planet.get_real_position();

            let system = game_data.systems.get(&planet.system_id).unwrap();

            let random = rand::thread_rng().gen_range(0..20);

            commands.spawn((
                Planet(planet_id.clone()),
                SpriteBundle {
                    texture: asset_server.load(SPRITES[random]),
                    transform: Transform {
                        translation: vec3(x as f32, y as f32, 1.),
                        scale: vec3(0.1, 0.1, 1.),
                        ..default()
                    },

                    visibility: Visibility::Hidden,
                    ..default()
                },
            ));

            commands.spawn((
                LabelComponent(LabelType::Planet),
                Text2dBundle {
                    text: Text::from_section(planet.name.clone(), text_style.clone())
                        .with_justify(JustifyText::Center),
                    transform: Transform::from_translation(Vec3::new(x as f32, y as f32 + 50., 3.)),
                    visibility: Visibility::Hidden,
                    ..default()
                },
            ));

            commands.spawn((
                Planet(planet_id.clone()),
                MaterialMesh2dBundle {
                    mesh: meshes.add(Annulus::new(i * 50.0, i * 50.0 + 5.0)).into(),
                    material: materials.add(Color::srgba(1., 1., 1., 0.1)),
                    transform: Transform {
                        translation: vec3(system.x as f32, system.y as f32, 0.),
                        scale: Vec3::new(1., 0.5, 1.),
                        ..default()
                    },
                    ..default()
                },
            ));

            nb_spawned += 1;
            loading.infos.write().unwrap().progress = nb_spawned as f32 / total_planets as f32;

            i += 1.;
        }
    }

    state.set(AppState::InGame);
}
pub fn spawn_links(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    loading: ResMut<LoadingWindow>,
) {
    let game_data = GAME_DATA.read();
    let mut drawn_links = BTreeSet::new();

    let total_links = game_data
        .systems
        .iter()
        .map(|(_, system)| system.links.len())
        .sum::<usize>();

    loading.infos.write().unwrap().message = "Spawning links".to_string();
    loading.infos.write().unwrap().progress = 0.;

    let mut nb_spawned = 0;

    for (_system_id, system) in &game_data.systems {
        for link in &system.links {
            if drawn_links.contains(&(link.clone(), system.id.clone()))
                || drawn_links.contains(&(system.id.clone(), link.clone()))
            {
                nb_spawned += 1;
                loading.infos.write().unwrap().progress = nb_spawned as f32 / total_links as f32;
                continue;
            }

            drawn_links.insert((system.id.clone(), link.clone()));

            let other_system = game_data.systems.get(&link).unwrap();

            let distance = system.distance_to(other_system);
            let distance_x = other_system.x - system.x;
            let distance_y = other_system.y - system.y;

            let rectangle = Rectangle::new(distance as f32 - SYSTEM_RADIUS, 100.);

            let angle = (distance_y as f32).atan2(distance_x as f32);

            commands.spawn((
                Link,
                MaterialMesh2dBundle {
                    mesh: meshes.add(rectangle).into(),
                    material: materials.add(Color::srgba(0., 0., 1., 1.)),
                    transform: Transform {
                        translation: vec3(
                            (system.x + distance_x / 2) as f32,
                            (system.y + distance_y / 2) as f32,
                            0.,
                        ),
                        rotation: Quat::from_rotation_z(angle),
                        ..default()
                    },
                    ..default()
                },
            ));

            nb_spawned += 1;
            loading.infos.write().unwrap().progress = nb_spawned as f32 / total_links as f32;
        }
    }
}
