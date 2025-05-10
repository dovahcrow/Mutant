use bevy_egui::egui::{self, Color32, Response};
use egui_dock::egui::Margin;
use egui_plot::{BoxElem, BoxPlot, BoxSpread, GridMark, Legend, Plot, PlotPoint, PlotPoints};
use emath::Vec2b;
use log::info;
use ogame_core::{
    game::Game,
    market::{Candlestick, CandlestickScale},
};
use std::collections::BTreeMap;

use super::utils::format_timestamp;

pub struct CandlestickChart<'a> {
    candlesticks: &'a BTreeMap<
        String,
        BTreeMap<String, BTreeMap<CandlestickScale, BTreeMap<String, Candlestick>>>,
    >,
    selected_item: &'a str,
    selected_scale: &'a CandlestickScale,
    height: f32,
}

impl<'a> CandlestickChart<'a> {
    pub fn new(
        candlesticks: &'a BTreeMap<
            String,
            BTreeMap<String, BTreeMap<CandlestickScale, BTreeMap<String, Candlestick>>>,
        >,
        selected_item: &'a str,
        selected_scale: &'a CandlestickScale,
        height: f32,
    ) -> Self {
        Self {
            candlesticks,
            selected_item,
            selected_scale,
            height,
        }
    }

    pub fn show(&self, ui: &mut egui::Ui) -> Response {
        let (candlestick_data, volumes) = self.prepare_chart_data();
        let (ups, downs) = self.split_candlesticks(candlestick_data.clone());
        let (price_max, volume_max) = self.get_max_values(&volumes);

        let downs_plot = BoxPlot::new(downs).color(Color32::from_rgb(255, 0, 0));
        let ups_plot = BoxPlot::new(ups).color(Color32::from_rgb(0, 255, 0));

        // Calculate MA and SMA lines
        let (ma_points, sma_points) = self.calculate_moving_averages(&candlestick_data);

        // Common plot settings
        let x_range = (0.0, (volumes.len() - 1) as f64);

        // Calculate the width needed for both price and volume
        let price_width = format!("{:.2}", price_max).len();
        let volume_width = format!("{:.0}", volume_max).len();
        let max_width = price_width.max(volume_width);

        ui.vertical(|ui| {
            // Price chart (taking 80% of total height)
            let price_height = self.height * 0.8;
            Plot::new("Price Plot")
                .legend(Legend::default())
                .allow_zoom(false)
                .allow_drag(false)
                .allow_scroll(false)
                .include_x(x_range.0)
                .include_x(x_range.1)
                .include_y(0.0)
                .include_y(price_max * 1.1)
                .show_grid([true, true])
                .x_axis_formatter(|mark, _range| {
                    format_timestamp(
                        self.get_first_timestamp(),
                        mark.value,
                        self.selected_scale.clone(),
                    )
                })
                .y_axis_formatter(move |value, _range| {
                    format!("{:>width$.2}", value.value, width = max_width)
                })
                .allow_boxed_zoom(false)
                .show_axes(true)
                .sharp_grid_lines(false)
                .height(price_height)
                .data_aspect(0.0)
                .show(ui, |plot_ui| {
                    // Plot candlesticks
                    plot_ui.box_plot(downs_plot);
                    plot_ui.box_plot(ups_plot);

                    // Plot MA and SMA lines
                    plot_ui.line(
                        egui_plot::Line::new(ma_points)
                            .color(Color32::from_rgb(255, 165, 0)) // Orange for MA
                            .name("MA (10)"),
                    );
                    plot_ui.line(
                        egui_plot::Line::new(sma_points)
                            .color(Color32::from_rgb(147, 112, 219)) // Purple for SMA
                            .name("SMA (20)"),
                    );
                });

            ui.add_space(4.0);

            // Volume chart (taking 20% of total height)
            let volume_height = self.height * 0.18; // 18% to account for spacing
            Plot::new("Volume Plot")
                .legend(Legend::default())
                .allow_zoom(false)
                .allow_drag(false)
                .allow_scroll(false)
                .include_x(x_range.0)
                .include_x(x_range.1)
                .include_y(0.0)
                .include_y(volume_max * 1.1)
                .show_grid([true, true])
                .x_axis_formatter(|mark, _range| {
                    format_timestamp(
                        self.get_first_timestamp(),
                        mark.value,
                        self.selected_scale.clone(),
                    )
                })
                .y_axis_formatter(move |value, _range| {
                    format!("{:>width$.0}", value.value, width = max_width)
                })
                .allow_boxed_zoom(false)
                .show_axes(true)
                .sharp_grid_lines(false)
                .height(volume_height)
                .data_aspect(0.0)
                .show(ui, |plot_ui| {
                    // Plot volume bars
                    let scaled_volumes: Vec<_> = volumes
                        .iter()
                        .enumerate()
                        .map(|(i, &vol)| egui_plot::Bar::new(i as f64, vol as f64).width(0.7))
                        .collect();

                    plot_ui.bar_chart(
                        egui_plot::BarChart::new(scaled_volumes)
                            .color(Color32::from_rgb(100, 100, 255)),
                    );
                });
        })
        .response
    }

    fn prepare_chart_data(&self) -> (Vec<(usize, Candlestick)>, Vec<i32>) {
        let empty = BTreeMap::new();
        let empty2 = BTreeMap::new();
        let default = vec![(
            "0".to_string(),
            Candlestick {
                id: "0".to_string(),
                market_id: "".to_string(),
                item_id: self.selected_item.to_string(),
                scale: self.selected_scale.clone(),
                open: 0,
                close: 0,
                high: 0,
                low: 0,
                volume: 0,
                timestamp: Game::now() as i32,
            },
        )]
        .into_iter()
        .collect();

        let market_data = self.candlesticks.values().next().unwrap_or(&empty);
        let item_data = market_data.get(self.selected_item).unwrap_or(&empty2);
        let scale_data = item_data.get(self.selected_scale).unwrap_or(&default);

        let mut candlestick_data = Vec::new();
        let mut volumes = Vec::new();
        let mut last_timestamp = scale_data.values().next().map(|c| c.timestamp).unwrap_or(0);
        let mut last_candlestick = scale_data.values().next().cloned();
        let mut idx = 0;

        // Fill in gaps and collect data
        for candlestick in scale_data.values() {
            while last_timestamp + (self.selected_scale.to_seconds() as i32) < candlestick.timestamp
            {
                if let Some(last) = &last_candlestick {
                    let last_close = last.close;
                    let gap_candlestick = Candlestick {
                        id: format!("gap_{}", idx),
                        market_id: last.market_id.clone(),
                        item_id: last.item_id.clone(),
                        scale: last.scale.clone(),
                        open: last_close,  // Use last closing price
                        close: last_close, // Use last closing price
                        high: last_close,  // Same as open/close
                        low: last_close,   // Same as open/close
                        volume: 0,
                        timestamp: last_timestamp + self.selected_scale.to_seconds() as i32,
                    };
                    candlestick_data.push((idx, gap_candlestick));
                    volumes.push(0);
                }
                last_timestamp += self.selected_scale.to_seconds() as i32;
                idx += 1;
            }

            candlestick_data.push((idx, candlestick.clone()));
            volumes.push(candlestick.volume as i32);

            last_timestamp = candlestick.timestamp;
            last_candlestick = Some(candlestick.clone());
            idx += 1;
        }

        // Fill remaining gaps until current time
        while last_timestamp + (self.selected_scale.to_seconds() as i32) < Game::now() as i32 {
            if let Some(last) = &last_candlestick {
                let last_close = last.close;
                let gap_candlestick = Candlestick {
                    id: format!("gap_{}", idx),
                    market_id: last.market_id.clone(),
                    item_id: last.item_id.clone(),
                    scale: last.scale.clone(),
                    open: last_close,  // Use last closing price
                    close: last_close, // Use last closing price
                    high: last_close,  // Same as open/close
                    low: last_close,   // Same as open/close
                    volume: 0,
                    timestamp: last_timestamp + self.selected_scale.to_seconds() as i32,
                };
                candlestick_data.push((idx, gap_candlestick));
                volumes.push(0);
            }
            last_timestamp += self.selected_scale.to_seconds() as i32;
            idx += 1;
        }

        (candlestick_data, volumes)
    }

    fn split_candlesticks(&self, data: Vec<(usize, Candlestick)>) -> (Vec<BoxElem>, Vec<BoxElem>) {
        let mut ups = Vec::new();
        let mut downs = Vec::new();
        let mut last_was_up = false;

        for (i, candlestick) in data {
            let mut elem = BoxElem::new(
                i as f64,
                BoxSpread::new(
                    candlestick.low as f64 / 100.0,
                    candlestick.open as f64 / 100.0,
                    candlestick.close as f64 / 100.0,
                    candlestick.close as f64 / 100.0,
                    candlestick.high as f64 / 100.0,
                ),
            );
            elem = elem.box_width(0.7);

            // For interpolated data, open equals close, so we use the previous candlestick's color
            if candlestick.open == candlestick.close && candlestick.volume == 0 {
                if last_was_up {
                    ups.push(elem);
                } else {
                    downs.push(elem);
                }
            } else {
                if candlestick.open < candlestick.close {
                    ups.push(elem);
                    last_was_up = true;
                } else {
                    downs.push(elem);
                    last_was_up = false;
                }
            }
        }

        (ups, downs)
    }

    fn get_max_values(&self, volumes: &[i32]) -> (f64, f64) {
        let empty = BTreeMap::new();
        let empty2 = BTreeMap::new();
        let market_data = self.candlesticks.values().next().unwrap_or(&empty);
        let item_data = market_data.get(self.selected_item).unwrap_or(&empty2);

        if let Some(scale_data) = item_data.get(self.selected_scale) {
            let price_max = scale_data.values().map(|c| c.high).max().unwrap_or(0) as f64 / 100.0;
            let volume_max = volumes.iter().copied().max().unwrap_or(0) as f64;
            (price_max, volume_max)
        } else {
            (0.0, 0.0)
        }
    }

    fn get_first_timestamp(&self) -> i32 {
        let empty = BTreeMap::new();
        let empty2 = BTreeMap::new();
        let market_data = self.candlesticks.values().next().unwrap_or(&empty);
        let item_data = market_data.get(self.selected_item).unwrap_or(&empty2);

        item_data
            .get(self.selected_scale)
            .and_then(|data| data.values().next())
            .map(|c| c.timestamp)
            .unwrap_or(0)
    }

    fn calculate_moving_averages(
        &self,
        candlesticks: &[(usize, Candlestick)],
    ) -> (Vec<[f64; 2]>, Vec<[f64; 2]>) {
        let ma_period = 10; // 10-period MA
        let sma_period = 20; // 20-period SMA

        let mut ma_points = Vec::new();
        let mut sma_points = Vec::new();

        // Calculate MA (10-period)
        for i in ma_period..candlesticks.len() {
            let ma_slice = &candlesticks[i - ma_period + 1..=i];
            let ma = ma_slice.iter().map(|(_, c)| c.close as f64).sum::<f64>() / ma_period as f64;
            ma_points.push([candlesticks[i].0 as f64, ma / 100.0]);
        }

        // Calculate SMA (20-period)
        for i in sma_period..candlesticks.len() {
            let sma_slice = &candlesticks[i - sma_period + 1..=i];
            let sma =
                sma_slice.iter().map(|(_, c)| c.close as f64).sum::<f64>() / sma_period as f64;
            sma_points.push([candlesticks[i].0 as f64, sma / 100.0]);
        }

        (ma_points, sma_points)
    }
}
