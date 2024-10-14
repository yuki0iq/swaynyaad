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

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name_fn(|| {
            use core::sync::atomic::{AtomicUsize, Ordering};
            static ATOMIC_ID: AtomicUsize = AtomicUsize::new(0);
            let id = ATOMIC_ID.fetch_add(1, Ordering::SeqCst);
            format!("nya-{}", id)
        })
        .build()
        .expect("create custom tokio runtime");
    let _guard = runtime.enter();
    info!("Entered tokio runtime from main thread");

    let app = relm4::main_application();
    app.set_application_id(Some("sylfn.swaynyaad.Bar"));
    debug!("Created gtk::Application");

    let start = std::sync::Once::new();
    app.connect_activate(move |app| {
        debug!("Received activate signal");
        let app = app.to_owned();
        start.call_once(move || {
            debug!("Starting relm4");
            std::mem::forget(app.hold());

            relm4::set_global_css(include_str!(concat!(env!("OUT_DIR"), "/style.css")));
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
