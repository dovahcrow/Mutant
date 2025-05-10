use bevy::{
    input::ButtonInput,
    math::Vec2,
    prelude::{MouseButton, Query, Res},
};
use ogame_core::{PositionedEntity, GAME_DATA};

use crate::app::{new_window, planet::PlanetWindow};

use super::{setup::scene::PLANET_RADIUS, CursorWorldPosition, Planet};

pub fn select_planet(
    buttons: Res<ButtonInput<MouseButton>>,
    mycoords: Res<CursorWorldPosition>,
    planets: Query<&Planet>,
) {
    if !buttons.just_released(MouseButton::Left) {
        return;
    }

    let game_data = GAME_DATA.read();

    let new_selected_planet = planets.iter().find(|planet| {
        let planet = game_data.get_planet(&planet.0).unwrap();
        let (x, y) = planet.get_real_position();

        let planet_position = Vec2::new(x as f32, y as f32);

        mycoords.0.distance(planet_position) < PLANET_RADIUS
    });

    let Some(new_selected_planet) = new_selected_planet else {
        return;
    };

    new_window(PlanetWindow {
        planet_id: new_selected_planet.0.clone(),
    });
}
