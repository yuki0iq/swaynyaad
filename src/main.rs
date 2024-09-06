use gtk::{glib, prelude::*};
use log::{debug, error, info};
use relm4::prelude::*;

mod app;
mod bar;
mod changer;
mod critical;
mod listeners;
mod state;

fn main() -> glib::ExitCode {
    env_logger::init();
    info!("swaynyaad is starting");

    let app = gtk::Application::builder()
        .application_id("sylfn.swaynyaad.Bar")
        .build();
    debug!("Created gtk::Application");

    let start = std::sync::Once::new();
    app.connect_activate(move |app| {
        debug!("Received activate signal");
        let app = app.to_owned();
        start.call_once(move || {
            debug!("Starting relm4");
            std::mem::forget(app.hold());

            relm4::set_global_css(grass::include!("src/style.scss"));
            relm4::spawn_local(async move {
                debug!("Entering main loop...");
                if let Err(e) = app::main_loop().await {
                    error!("Main loop: {e:?}");
                    std::process::abort();
                }
            });
        });
    });

    app.run()
}
