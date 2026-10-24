//! Saved layer preference only: changing this control never remaps the scene.
use super::{Options, state::AppState, style};
use crate::{config::WaylandLayer, desktop::layer};
use anyhow::Result;
use gtk::prelude::*;
use std::{cell::RefCell, rc::Rc};

pub(super) fn available(options: &Options) -> bool {
    cfg!(feature = "wayland-desktop")
        && options.desktop
        && options.experimental_wayland
        && !options.shell_surface
        && !options.benchmark
}

pub(super) fn controls(options: &Options, state: Rc<RefCell<AppState>>) -> Option<gtk::Box> {
    available(options).then(|| build(state, layer::environment_override()))
}

fn description(layer: WaylandLayer) -> &'static str {
    match layer {
        WaylandLayer::Automatic => {
            "Chooses a layer for your desktop. Changes apply after restarting Talis."
        }
        WaylandLayer::Background => {
            "Stays below bar overlays. A restarted wallpaper may cover Talis. Restart Talis to apply."
        }
        WaylandLayer::Bottom => {
            "Stays above background wallpapers. May cover bar overlays. Restart Talis to apply."
        }
    }
}

fn build(state: Rc<RefCell<AppState>>, environment: Result<Option<WaylandLayer>>) -> gtk::Box {
    let choice = gtk::ComboBoxText::new();
    choice.set_widget_name("desktop-layer");
    for (layer, label) in [
        (WaylandLayer::Automatic, "Automatic"),
        (WaylandLayer::Background, "Background"),
        (WaylandLayer::Bottom, "Bottom"),
    ] {
        choice.append(Some(layer.id()), label);
    }
    let preference = state.borrow().config.render.wayland_layer;
    let (row, detail) = style::row_with_detail("Desktop layer", "", &choice);
    detail.set_widget_name("desktop-layer-detail");
    detail.set_visible(true);
    match environment {
        Ok(None) => {
            choice.set_active_id(Some(preference.id()));
            detail.set_text(description(preference));
            choice.connect_changed(move |choice| {
                if !choice.is_sensitive() {
                    return;
                }
                if let Some(layer) = choice
                    .active_id()
                    .as_deref()
                    .and_then(WaylandLayer::from_id)
                {
                    state.borrow_mut().config.render.wayland_layer = layer;
                    detail.set_text(description(layer));
                }
            });
        }
        Ok(Some(layer)) => {
            choice.set_active_id(Some(layer.id()));
            choice.set_sensitive(false);
            detail.set_text(
                "Set by TALIS_WAYLAND_LAYER. Restart Talis without this override to change it.",
            );
        }
        Err(error) => {
            choice.set_active_id(Some(preference.id()));
            choice.set_sensitive(false);
            detail.set_text(&format!("Desktop layer unavailable: {error:#}"));
        }
    }
    row
}

#[cfg(test)]
mod tests;
