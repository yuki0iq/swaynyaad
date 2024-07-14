use gtk4::glib;
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use relm4::view;
use smol::stream::StreamExt;
use smol::Timer;
use std::time::Duration;

fn main() -> glib::ExitCode {
    let app = gtk4::Application::builder()
        .application_id("sylfn.SwayNyaaBar")
        .build();

    app.connect_activate(build_ui);

    app.run()
}

fn build_ui(app: &gtk4::Application) {
    view! {
        gtk4::ApplicationWindow::new(app) {
            init_layer_shell: (),
            auto_exclusive_zone_enable: (),
            set_layer: Layer::Top,
            set_anchor: (Edge::Bottom, true),
            set_anchor: (Edge::Left, true),
            set_anchor: (Edge::Right, true),

            gtk4::Box {
                set_orientation: gtk4::Orientation::Horizontal,

                container_add: label = &gtk4::Label {},
            },

            present: (),
        },
    };

    glib::spawn_future_local(glib::clone!(
        @weak label => async move {
            Timer::interval(Duration::from_secs(1))
                .for_each(|instant| {
                    label.set_text(&format!("{instant:#?}"));
                })
                .await;
        }
    ));
}
