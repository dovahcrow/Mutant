use bevy::{
    ecs::query::QueryFilter,
    prelude::{Camera, OrthographicProjection, Query, Transform, Visibility, With, Without},
};

use super::{LabelComponent, LabelType, Link, Planet, System};

pub fn update_visibility_with_scale(
    mut projection: Query<&mut OrthographicProjection, With<Camera>>,
    planets: Query<&mut Visibility, (With<Planet>, Without<System>, Without<Link>)>,
    labels: Query<
        (&mut Visibility, &mut Transform, &LabelComponent),
        (Without<Link>, Without<System>, Without<Planet>),
    >,
) {
    let Ok(mut projection) = projection.get_single_mut() else {
        return;
    };

    if projection.scale < 50. {
        toggle_visibility(true, planets);
    } else if projection.scale > 50. {
        toggle_visibility(false, planets);
    }

    if projection.scale < 20. {
        toggle_label_visibility(true, &mut projection, labels);
    } else if projection.scale > 20. {
        toggle_label_visibility(false, &mut projection, labels);
    }
}

fn toggle_visibility<Filter: QueryFilter>(
    visible: bool,
    mut query: Query<&mut Visibility, Filter>,
) {
    for mut vis_map in query.iter_mut() {
        if visible {
            *vis_map = Visibility::Visible;
        } else {
            *vis_map = Visibility::Hidden;
        }
    }
}

fn toggle_label_visibility(
    visible: bool,
    projection: &mut OrthographicProjection,
    mut query: Query<
        (&mut Visibility, &mut Transform, &LabelComponent),
        (Without<Link>, Without<System>, Without<Planet>),
    >,
) {
    for (mut vis_map, mut transform, label) in query.iter_mut() {
        match label.0 {
            LabelType::System => {
                transform.scale = (projection.scale * 0.1, projection.scale * 0.1, 1.).into();
            }
            LabelType::Planet => {
                transform.scale = (projection.scale * 0.2, projection.scale * 0.2, 1.).into();
            }
        }
        if LabelType::Planet == label.0 {
            if visible {
                *vis_map = Visibility::Visible;
            } else {
                *vis_map = Visibility::Hidden;
            }
        }
    }
}
