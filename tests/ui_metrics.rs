//! L1: compares `UiStyle::zed()` with the Zed baseline in `metrics.toml`.

use gpui::{Pixels, Rems};
use serde::Deserialize;
use yttt::ui::primitives::tabs::yttt_tabbar_style;
use yttt_ui::{
    style::{UiStyle, UiStyleId},
    theme::WorkbenchTheme,
};

const REM: f32 = 16.0;

#[derive(Deserialize)]
struct Metrics {
    metric: Vec<Metric>,
}

#[derive(Deserialize)]
struct Metric {
    name: String,
    zed: f32,
    aligned: bool,
}

fn rems_px(value: Rems) -> f32 {
    value.0 * REM
}

fn yttt_value(name: &str, style: UiStyle) -> Option<f32> {
    let tabs = yttt_tabbar_style(WorkbenchTheme::one_dark(), style);
    let pixels = |value: Pixels| Some(f32::from(value));
    match name {
        "button_height_px" => Some(rems_px(style.controls.button_height)),
        "tab_height_px" => Some(rems_px(tabs.height)),
        "tab_radius_px" => pixels(style.rows.tab_radius),
        "text_ui_px" => Some(rems_px(style.text.default)),
        "palette_width_px" => pixels(style.palette.panel_width),
        _ => None,
    }
}

#[test]
fn zed_style_matches_the_measured_baseline() {
    let metrics: Metrics =
        toml::from_str(include_str!("visual/zed/metrics.toml")).expect("metrics parse");
    let style = UiStyle::resolve(UiStyleId::Zed);

    for metric in metrics.metric.iter().filter(|metric| metric.aligned) {
        let value = yttt_value(&metric.name, style)
            .unwrap_or_else(|| panic!("no yttt value for {}", metric.name));
        assert_eq!(value, metric.zed, "{}", metric.name);
    }
}
