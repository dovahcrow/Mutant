use bevy::prelude::{Camera2dBundle, Commands};

use crate::cam_test::PanCam;

pub fn setup_camera(mut commands: Commands) {
    let camerabundle = Camera2dBundle::default();
    commands.spawn(camerabundle).insert(PanCam::default());
}
