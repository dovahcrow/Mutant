use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};

use bevy_egui::egui;
use egui_dock::egui::{Pos2, Rect, Sense, Shape, Stroke, Vec2};
use log::info;
use node_shape::NodeShape;
use ogame_core::{protocol::Protocol, BaseId, GAME_DATA};
use petgraph::{csr::DefaultIx, graph::NodeIndex, prelude::StableGraph, Directed};
// use petgraph::prelude::*;
use egui_graphs::{
    random_graph, DefaultEdgeShape, DefaultNodeShape, Graph, GraphView, LayoutHierarchical,
    LayoutRandom, LayoutStateHierarchical, LayoutStateRandom, SettingsInteraction,
    SettingsNavigation, SettingsStyle,
};

use crate::{game, game_mut};

use super::Window;

type PetgraphGraphType = StableGraph<String, String>;
type GraphType = Graph<String, String, Directed, DefaultIx, NodeShape, DefaultEdgeShape>;

use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
pub struct ResearchWindow {
    pub selected_base: Option<BaseId>,
}

impl Window for ResearchWindow {
    fn name(&self) -> String {
        "Research".to_string()
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        self.test2(ui);
        // ui.add(&mut DefaultGraphView::new(&mut self.graph));

        self.base_selector(ui);

        let all_technologies = GAME_DATA.read().technologies.clone();
        let done_technologies = game().technologies.clone();
        let current_research = game().research.clone();

        // get researchable technologies
        let researchable_technologies = all_technologies
            .iter()
            .filter(|(tech_id, tech)| {
                let is_not_current_research = current_research
                    .clone()
                    .map(|r| r.name.clone() != (*tech_id).clone())
                    .unwrap_or(true);
                let is_not_done = !done_technologies.contains(tech_id);
                let is_unlocked = tech.dependencies.is_empty()
                    || tech
                        .dependencies
                        .iter()
                        .all(|prereq| done_technologies.contains(prereq));

                is_not_current_research && is_not_done && is_unlocked
            })
            .collect::<BTreeMap<_, _>>();

        for (tech_id, tech) in researchable_technologies {
            let all_that_will_unlock = all_technologies
                .iter()
                .filter(|(_, other_tech)| {
                    other_tech.dependencies.contains(tech_id)
                        && !done_technologies.contains(&other_tech.name.as_str().to_string())
                })
                .collect::<Vec<_>>();
            egui::CollapsingHeader::new(tech.name.clone())
                .open(None)
                .show(ui, |ui| {
                    ui.label(tech.description.clone());
                    ui.label(format!("Cost: {}", tech.cost));
                    ui.label(format!("Time: {}", tech.time));
                    //unlocks
                    if !all_that_will_unlock.is_empty() {
                        ui.label("Unlocks:");
                        for (_other_tech_id, other_tech) in all_that_will_unlock {
                            ui.label(other_tech.name.clone());
                        }
                    }
                    ui.add_enabled_ui(self.selected_base.is_some(), |ui| {
                        if ui.button("Research").clicked() {
                            let _ = game_mut().action(Protocol::Research {
                                technology_id: tech_id.clone(),
                                base_id: self.selected_base.clone().unwrap(),
                            });
                        }
                    });
                });
        }
    }
}
mod node_shape {
    use bevy_egui::egui::{
        epaint::{CircleShape, TextShape},
        Color32, FontFamily, FontId, Pos2, Rect, Shape, Stroke, Vec2,
    };
    use egui_dock::egui::{epaint::RectShape, Rounding};
    use log::info;
    use ogame_core::GAME_DATA;
    use petgraph::{stable_graph::IndexType, EdgeType};

    use egui_graphs::{DisplayNode, DrawContext, NodeProps};

    use crate::utils::game;

    /// This is the default node shape which is used to display nodes in the graph.
    ///
    /// You can use this implementation as an example for implementing your own custom node shapes.
    #[derive(Clone, Debug)]
    pub struct NodeShape {
        pub pos: Pos2,

        pub selected: bool,
        pub dragged: bool,
        pub color: Option<Color32>,

        pub label_text: String,

        /// Shape dependent property
        pub size: Pos2,
    }

    impl<N: Clone> From<NodeProps<N>> for NodeShape {
        fn from(node_props: NodeProps<N>) -> Self {
            info!("NodeProps: {:#?}", node_props.label);
            let color = GAME_DATA
                .read()
                .technologies
                .get(&node_props.label.to_string())
                .map(|tech| {
                    if game().technologies.contains(&node_props.label.to_string()) {
                        Color32::from_rgb(0, 255, 0)
                    } else {
                        Color32::from_rgb(255, 0, 0)
                    }
                });
            info!("Color: {:#?}", color);
            NodeShape {
                pos: node_props.location(),
                selected: node_props.selected,
                dragged: node_props.dragged,
                label_text: node_props.label.to_string(),
                color,

                size: Pos2::new(20.0, 10.0),
            }
        }
    }

    impl<N: Clone, E: Clone, Ty: EdgeType, Ix: IndexType> DisplayNode<N, E, Ty, Ix> for NodeShape {
        fn is_inside(&self, pos: Pos2) -> bool {
            is_inside_rect(self.pos, self.size, pos)
        }

        fn closest_boundary_point(&self, dir: Vec2) -> Pos2 {
            closest_point_on_rect(self.pos, self.size, dir)
        }

        fn shapes(&mut self, ctx: &DrawContext) -> Vec<Shape> {
            let mut res = Vec::with_capacity(2);

            let is_interacted = self.selected || self.dragged;

            let style = if is_interacted {
                ctx.ctx.style().visuals.widgets.active
            } else {
                ctx.ctx.style().visuals.widgets.inactive
            };

            let color = if let Some(c) = self.color {
                c
            } else {
                style.fg_stroke.color
            };

            let rect_center = ctx.meta.canvas_to_screen_pos(self.pos);
            let rect_size_x = ctx.meta.canvas_to_screen_size(self.size.x);
            let rect_size_y = ctx.meta.canvas_to_screen_size(self.size.y);
            let rect_size = Vec2::new(rect_size_x, rect_size_y);

            let rect_center = Pos2::new(rect_center.x, rect_center.y);

            let rect =
                Rect::from_two_pos(rect_center - rect_size / 2.0, rect_center + rect_size / 2.0);

            let rectangle_shape = RectShape::stroke(rect, Rounding::ZERO, Stroke::new(1.0, color));

            res.push(rectangle_shape.into());

            let font_size = ctx.meta.canvas_to_screen_size(3.0);

            let galley = ctx.ctx.fonts(|f| {
                f.layout_no_wrap(
                    self.label_text.clone(),
                    FontId::new(font_size, FontFamily::Monospace),
                    color,
                )
            });

            // display label centered over the rect
            let label_pos = Pos2::new(
                rect_center.x - galley.size().x / 2.0,
                rect_center.y - galley.size().y * 3.0,
            );

            let label_shape = TextShape::new(label_pos, galley, color);
            res.push(label_shape.into());

            res
        }

        fn update(&mut self, state: &NodeProps<N>) {
            let color = GAME_DATA
                .read()
                .technologies
                .get(&state.label.to_string())
                .map(|tech| {
                    if game().technologies.contains(&state.label.to_string()) {
                        Color32::from_rgb(0, 255, 0)
                    } else if let Some(research) = &game().research {
                        if research.name == state.label.to_string() {
                            Color32::from_rgb(0, 0, 255)
                        } else {
                            Color32::from_rgb(255, 0, 0)
                        }
                    } else {
                        Color32::from_rgb(255, 0, 0)
                    }
                });
            self.pos = state.location();
            self.selected = state.selected;
            self.dragged = state.dragged;
            self.label_text = state.label.to_string();
            self.color = color;
        }
    }

    fn is_inside_rect(pos: Pos2, size: Pos2, to_test: Pos2) -> bool {
        to_test.x >= pos.x - size.x
            && to_test.x <= pos.x + size.x
            && to_test.y >= pos.y - size.y
            && to_test.y <= pos.y + size.y
    }

    fn closest_point_on_rect(pos: Pos2, size: Pos2, dir: Vec2) -> Pos2 {
        let mut res = pos;

        if dir.x < 0.0 {
            res.x -= size.x;
        }

        if dir.y < 0.0 {
            res.y -= size.y;
        }

        res
    }
}

lazy_static::lazy_static! {
    static ref GRAPH: Arc<RwLock<GraphType>> = Arc::new(RwLock::new(ResearchWindow::generate_graph()));
}

impl ResearchWindow {
    pub fn new() -> Self {
        Self {
            selected_base: None,
        }
    }

    pub fn regen_graph() {
        *GRAPH.write().unwrap() = Self::generate_graph();
    }

    fn test2(&mut self, ui: &mut egui::Ui) {
        let interaction_settings = &SettingsInteraction::new()
            .with_node_clicking_enabled(true)
            .with_node_selection_enabled(true)
            .with_node_selection_multi_enabled(true)
            .with_edge_clicking_enabled(true)
            .with_edge_selection_enabled(true)
            .with_edge_selection_multi_enabled(true);
        ui.add(
            &mut GraphView::<
                _,
                _,
                _,
                _,
                node_shape::NodeShape,
                _,
                LayoutStateHierarchical,
                LayoutHierarchical,
            >::new(&mut GRAPH.write().unwrap())
            .with_interactions(interaction_settings)
            .with_navigations(
                &SettingsNavigation::default()
                    .with_fit_to_screen_enabled(false)
                    .with_zoom_speed(0.01)
                    .with_zoom_and_pan_enabled(true),
            ),
        );
    }
    fn generate_graph() -> GraphType {
        let techs = GAME_DATA.read().technologies.clone();
        let mut graph = PetgraphGraphType::new();
        let mut node_ids = BTreeMap::new();

        let mut sorted_techs = techs.clone().into_iter().collect::<Vec<_>>();
        sorted_techs.sort_by(|a, b| a.1.dependencies.len().cmp(&b.1.dependencies.len()));

        // filter by researchable
        let techs = techs
            .into_iter()
            .filter(|(tech_id, tech)| {
                let is_unlocked = tech.dependencies.is_empty()
                    || tech
                        .dependencies
                        .iter()
                        .all(|prereq| game().technologies.contains(prereq));
                is_unlocked
            })
            .collect::<BTreeMap<_, _>>();

        for (name, tech) in &techs {
            Self::recur_insert(&techs, &mut graph, &mut node_ids, name.clone());
        }

        let mut graph = Graph::from(&graph);

        Self::set_labels(&mut graph, &node_ids);

        graph
    }

    fn recur_insert(
        techs: &BTreeMap<String, ogame_core::Technology>,
        graph: &mut PetgraphGraphType,
        node_ids: &mut BTreeMap<String, petgraph::graph::NodeIndex>,
        tech_id: String,
    ) -> NodeIndex {
        if node_ids.contains_key(&tech_id) {
            return *node_ids.get(&tech_id).unwrap();
        }
        let id = graph.add_node(tech_id.clone());
        node_ids.insert(tech_id.clone(), id);
        for dep in techs.get(&tech_id).unwrap().dependencies.clone() {
            if let Some(tech_id) = node_ids.get(dep.as_str()) {
                graph.add_edge(*tech_id, id, dep.clone());
            } else {
                let tech_id = Self::recur_insert(techs, graph, node_ids, dep.clone());
                graph.add_edge(tech_id, id, dep.clone());
            }
        }

        id
    }

    fn set_labels(graph: &mut GraphType, node_ids: &BTreeMap<String, petgraph::graph::NodeIndex>) {
        for (tech_id, node_id) in node_ids {
            graph.node_mut(*node_id).unwrap().set_label(tech_id.clone());
        }
    }

    fn base_selector(&mut self, ui: &mut egui::Ui) {
        let bases = game().bases.clone();
        ui.horizontal(|ui| {
            ui.label("Base:");

            for (base_id, base) in bases {
                let planet_name = GAME_DATA
                    .read()
                    .get_planet(&base.planet_id)
                    .unwrap()
                    .name
                    .clone();
                if ui.button(planet_name).clicked() {
                    self.selected_base = Some(base_id);
                }
            }
        });
    }
}
