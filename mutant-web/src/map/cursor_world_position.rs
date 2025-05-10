use bevy::{
    prelude::{Camera, GlobalTransform, Query, ResMut, With},
    window::{PrimaryWindow, Window},
};

use super::CursorWorldPosition;

pub fn update_cursor_world_position(
    mut mycoords: ResMut<CursorWorldPosition>,
    q_window: Query<&Window, With<PrimaryWindow>>,
    q_camera: Query<(&Camera, &GlobalTransform), With<Camera>>,
) {
    let (camera, camera_transform) = q_camera.single();

    let window = q_window.single();

    if let Some(world_position) = window
        .cursor_position()
        .and_then(|cursor| camera.viewport_to_world(camera_transform, cursor))
        .map(|ray| ray.origin.truncate())
    {
        mycoords.0 = world_position;
    }
}
