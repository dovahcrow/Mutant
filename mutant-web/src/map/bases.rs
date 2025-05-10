use std::collections::BTreeMap;

use bevy::{
    asset::Assets,
    color::Color,
    math::{vec3, Vec2},
    prelude::{
        Camera, Commands, Entity, Mesh, OrthographicProjection, Query, Rectangle, ResMut,
        Transform, Triangle2d, With,
    },
    sprite::{ColorMaterial, MaterialMesh2dBundle},
    utils::default,
};
use ogame_core::PositionedEntity;

use crate::utils::game;

use super::BaseComponent;

pub fn draw_bases(
    bases: Query<(Entity, &BaseComponent, &mut Transform)>,
    projection: Query<&mut OrthographicProjection, With<Camera>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let Ok(projection) = projection.get_single() else {
        return;
    };

    let game = game();

    let game_bases = game
        .bases
        .iter()
        .chain(
            game.other_players
                .iter()
                .flat_map(|(_, player)| player.bases.iter()),
        )
        .collect::<BTreeMap<_, _>>();

    for (base_id, base) in &game_bases {
        if bases
            .iter()
            .find(|(_entity, base, _)| base.0 == **base_id)
            .is_none()
        {
            let (x, y) = base.get_real_position();

            // a triangle and a rectangle that form a little house
            let triangle = Triangle2d::new(
                Vec2::new(-70., 30.),
                Vec2::new(0., 70.),
                Vec2::new(70., 30.),
            );
            let rectangle = Rectangle::new(80., 60.);

            let color = if base.user_id == game.user_id {
                Color::srgba(0., 1., 0., 1.)
            } else {
                Color::srgba(1., 0., 0., 1.)
            };

            commands.spawn((
                BaseComponent((*base_id).clone()),
                MaterialMesh2dBundle {
                    mesh: meshes.add(triangle).into(),
                    material: materials.add(color),
                    transform: Transform {
                        translation: vec3(x as f32 - 20.0, y as f32 + 20., 3.),
                        ..default()
                    },
                    ..default()
                },
            ));

            commands.spawn((
                BaseComponent((*base_id).clone()),
                MaterialMesh2dBundle {
                    mesh: meshes.add(rectangle).into(),
                    material: materials.add(color),
                    transform: Transform {
                        translation: vec3(x as f32 - 20.0, y as f32 + 20., 3.),
                        ..default()
                    },
                    ..default()
                },
            ));
        }
    }

    for (entity, base, _) in bases.iter() {
        if game_bases.get(&base.0).is_none() {
            commands.entity(entity).despawn();
        }
    }

    // update scale
    for (entity, base, transform) in bases.iter() {
        let Some(_base) = game_bases.get(&base.0) else {
            continue;
        };
        commands.entity(entity).insert(Transform {
            scale: (projection.scale * 0.1, projection.scale * 0.1, 1.).into(),
            ..*transform
        });
    }
}
