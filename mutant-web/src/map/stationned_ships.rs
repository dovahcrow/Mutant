use std::collections::BTreeMap;

use bevy::{
    asset::Assets,
    color::Color,
    math::{vec3, Vec2},
    prelude::{
        Camera, Commands, Entity, Mesh, OrthographicProjection, Query, ResMut, Transform,
        Triangle2d, With,
    },
    sprite::{ColorMaterial, MaterialMesh2dBundle},
    utils::default,
};
use ogame_core::PositionedEntity;

use crate::utils::game;

use super::ShipComponent;

pub fn draw_stationned_ships(
    ships: Query<(Entity, &ShipComponent, &mut Transform)>,
    projection: Query<&mut OrthographicProjection, With<Camera>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let Ok(projection) = projection.get_single() else {
        return;
    };

    let game = game();

    let game_stationned_ships = game
        .ships
        .iter()
        .chain(
            game.other_players
                .iter()
                .flat_map(|(_, player)| player.ships.iter()),
        )
        .filter(|(_, ship)| ship.flight_id.is_none())
        .collect::<BTreeMap<_, _>>();

    for (ship_id, ship) in &game_stationned_ships {
        if ships
            .iter()
            .find(|(_entity, f, _)| f.0 == **ship_id)
            .is_none()
        {
            let (x, y) = ship.get_real_position();

            let triangle =
                Triangle2d::new(Vec2::new(0., 0.), Vec2::new(100., 0.), Vec2::new(50., 100.));

            let color = if ship.user_id == game.user_id {
                Color::srgba(0., 1., 0., 1.)
            } else {
                Color::srgba(1., 0., 0., 1.)
            };

            commands.spawn((
                ShipComponent((*ship_id).clone()),
                MaterialMesh2dBundle {
                    mesh: meshes.add(triangle).into(),
                    material: materials.add(color),
                    transform: Transform {
                        translation: vec3(x as f32 + 20.0, y as f32 + 20.0, 3.),
                        ..default()
                    },
                    ..default()
                },
            ));
        }
    }

    for (entity, ship, _) in ships.iter() {
        if game_stationned_ships.get(&ship.0).is_none() {
            commands.entity(entity).despawn();
        }
    }

    // update scale
    for (entity, ship, transform) in ships.iter() {
        let Some(_ship) = game_stationned_ships.get(&ship.0) else {
            continue;
        };
        commands.entity(entity).insert(Transform {
            scale: (projection.scale * 0.1, projection.scale * 0.1, 1.).into(),
            ..*transform
        });
    }
}
