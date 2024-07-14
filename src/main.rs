use anyhow::{Context, Result};
use gtk4::glib;
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use relm4::view;
use smol::stream::StreamExt;
// use smol::Timer;
// use std::time::Duration;

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
                set_spacing: 16,

                // container_add: time = &gtk4::Label {},

                container_add: layout = &gtk4::Label {
                    set_tooltip_text: Some("fuck you"),
                },
            },

            present: (),
        },
    };

    let (tx, rx) = async_channel::unbounded();
    glib::spawn_future_local(async move {
        let res = listen_sway_layout(tx).await.context("sway layout");
        eprintln!("{res:?}");
    });
    glib::spawn_future_local(glib::clone!(
        @weak layout => async move {
            while let Ok(state) = rx.recv().await {
                let name = state.name.unwrap_or_else(|| "XX:Unknown".into());
                let short_name = name[..2].to_ascii_lowercase();
                layout.set_text(&short_name);
                layout.set_tooltip_text(Some(&name));
            }
        }
    ));

    // glib::spawn_future_local(glib::clone!(
    //     @weak time => async move {
    //         Timer::interval(Duration::from_secs(1))
    //             .for_each(|instant| {
    //                 time.set_text(&format!("{instant:?}"));
    //             })
    //             .await;
    //     }
    // ));
}

struct Layout {
    name: Option<String>,
}

async fn listen_sway_layout(tx: async_channel::Sender<Layout>) -> Result<()> {
    let mut stream = swayipc_async::Connection::new()
        .await
        .context("create connection for subscribe")?
        .subscribe(&[swayipc_async::EventType::Input])
        .await
        .context("subscribe")?;
    let mut conn = swayipc_async::Connection::new()
        .await
        .context("could not create connection")?;
    while {
        // do
        let layout_name = conn
            .get_inputs()
            .await
            .context("Get inputs")?
            .into_iter()
            .find_map(|input| input.xkb_active_layout_name);
        tx.send(Layout { name: layout_name })
            .await
            .context("send updated state")?;

        // while
        stream.next().await.is_some()
    } {}
    Ok(())
}
