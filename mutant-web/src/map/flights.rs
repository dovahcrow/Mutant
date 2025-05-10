use bevy::{
    asset::Assets,
    color::Color,
    math::{vec3, Quat, Vec2},
    prelude::{
        Camera, Commands, Entity, Mesh, OrthographicProjection, Query, Rectangle, ResMut,
        Transform, Triangle2d, With, Without,
    },
    sprite::{ColorMaterial, MaterialMesh2dBundle},
    utils::default,
};
use ogame_core::{PositionedEntity, GAME_DATA};

use crate::utils::game;

use super::{FlightComponent, Link};

pub fn draw_flights(
    flights: Query<(Entity, &FlightComponent, &mut Transform), Without<Link>>,
    links: Query<(Entity, &FlightComponent), With<Link>>,
    projection: Query<&mut OrthographicProjection, With<Camera>>,

    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let Ok(projection) = projection.get_single() else {
        return;
    };
    // iterate over game flights, and compare with those in the query
    // remove the ones that are not in the game anymore
    // add the ones that are in the game but not in the query
    // update the ones that are in both

    let game_flights = game().flights.clone();

    for (flight_id, flight) in &game_flights {
        if flights
            .iter()
            .find(|(_entity, f, _)| f.0 == *flight_id)
            .is_none()
        {
            let from_system = GAME_DATA.read().get_entity(&flight.from_id).unwrap();
            let to_system = GAME_DATA.read().get_entity(&flight.to_id).unwrap();
            let (from_x, from_y) = from_system.get_real_position();
            let (to_x, to_y) = to_system.get_real_position();

            let position = flight.get_real_position();

            // triangle pointing to the top
            let triangle =
                Triangle2d::new(Vec2::new(0., 0.), Vec2::new(100., 0.), Vec2::new(50., 100.));

            let distance_x = to_x - from_x;
            let distance_y = to_y - from_y;

            let angle = (distance_y as f32).atan2(distance_x as f32);

            // add a quarter of a circle to the angle to make the triangle point to the right
            let ship_angle = angle - std::f32::consts::FRAC_PI_2;

            commands.spawn((
                FlightComponent(flight_id.clone()),
                MaterialMesh2dBundle {
                    mesh: meshes.add(triangle).into(),
                    material: materials.add(Color::srgba(0., 1., 1., 1.)),
                    transform: Transform {
                        translation: vec3(position.0 as f32, position.1 as f32, 3.),
                        rotation: Quat::from_rotation_z(ship_angle),
                        ..default()
                    },
                    ..default()
                },
            ));

            let distance = from_system.distance_to(&to_system) as i32;

            let rectangle = Rectangle::new(distance as f32, 200.);

            commands.spawn((
                Link,
                FlightComponent(flight_id.clone()),
                MaterialMesh2dBundle {
                    mesh: meshes.add(rectangle).into(),
                    material: materials.add(Color::srgba(0., 1., 1., 0.5)),
                    transform: Transform {
                        translation: vec3(
                            (to_x - distance_x / 2) as f32,
                            (to_y - distance_y / 2) as f32,
                            0.,
                        ),
                        rotation: Quat::from_rotation_z(angle),
                        ..default()
                    },
                    ..default()
                },
            ));
        }
    }

    for (entity, flight, _) in flights.iter() {
        if game_flights.get(&flight.0).is_none() {
            commands.entity(entity).despawn();
        }
    }

    for (entity, flight) in links.iter() {
        if game_flights.get(&flight.0).is_none() {
            commands.entity(entity).despawn();
        }
    }

    for (entity, flight, transform) in flights.iter() {
        //update the position of the flight
        let Some(flight) = game_flights.get(&flight.0) else {
            continue;
        };
        let position = flight.get_real_position();
        commands.entity(entity).insert(Transform {
            translation: vec3(position.0 as f32, position.1 as f32, 3.),
            scale: (projection.scale * 0.1, projection.scale * 0.1, 1.).into(),
            ..*transform
        });
    }
}
