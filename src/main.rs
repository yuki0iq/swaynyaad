use anyhow::{ensure, Context, Result};
use gtk::prelude::*;
use gtk::{gdk, glib};
use log::{debug, error, info, trace, warn};
use relm4::prelude::*;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Source};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

mod bar;
mod changer;
mod critical;
mod listeners;
mod state;

use crate::bar::{AppInput, AppModel};
use crate::state::AppState;

fn play_sound(
    stream_handle: &OutputStreamHandle,
    state: &AppState,
    event: &AppInput,
) -> Result<()> {
    let name = match event {
        AppInput::Pulse(_) => "audio-volume-change",
        AppInput::PowerChanged => {
            if state.power.charging {
                "power-plug"
            } else {
                "power-unplug"
            }
        }

        _ => return Ok(()),
    };

    debug!("Playing event {name} with rodio");

    let path = format!("/usr/share/sounds/freedesktop/stereo/{name}.oga");
    let file = std::io::BufReader::new(std::fs::File::open(path).context("open audio file")?);
    let source = Decoder::new(file).context("decode audio")?;
    stream_handle
        .play_raw(source.convert_samples())
        .context("play audio")?;

    Ok(())
}

async fn main_loop() -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let state = Arc::new(RwLock::new(AppState::default()));

    listeners::start(tx, Arc::clone(&state));

    let mut windows: HashMap<String, Controller<AppModel>> = HashMap::new();

    let (_stream, stream_handle) = OutputStream::try_default().context("create output stream")?;

    info!("Ready dispatching events");

    loop {
        let event = rx.recv().await.context("receive event")?;
        debug!("Received {event:?}");
        trace!("Current state is {:?}", state.read().unwrap());
        let AppInput::Outputs(new_outputs) = event else {
            play_sound(&stream_handle, &state.read().unwrap(), &event).context("play sound")?;

            // TODO use broadcast channels (drop relm4)
            for controller in windows.values() {
                controller.sender().emit(event.clone());
            }
            continue;
        };

        windows.retain(|output, _| new_outputs.contains(output));

        let monitors = gdk::Display::default()
            .context("Failed to get default display")?
            .monitors()
            .into_iter()
            .take_while(Result::is_ok)
            .flatten()
            .flat_map(|res| res.downcast::<gdk::Monitor>())
            .collect::<Vec<_>>();

        for added in new_outputs
            .iter()
            .filter(|&output| !windows.contains_key(output))
            .collect::<Vec<_>>()
        {
            let monitor = monitors
                .iter()
                .find(|monitor| monitor.connector().as_deref() == Some(added))
                .context("unknown monitor");
            let Ok(monitor) = monitor else {
                warn!(
                    "GDK and Sway monitor mismatch! {} exists, but not for GDK",
                    added
                );
                continue;
            };

            let controller = AppModel::builder()
                .launch(AppModel::create(Arc::clone(&state), monitor.clone()))
                .detach();

            ensure!(
                windows.insert(added.into(), controller).is_none(),
                "nonexistent element exists"
            );
        }
    }
}

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
            relm4::set_global_css(include_str!("style.css"));
            relm4::spawn_local(async move {
                debug!("Entering main loop...");
                if let Err(e) = main_loop().await {
                    error!("Main loop: {e:?}");
                    std::process::abort();
                }
            });
        });
    });

    app.run()
}
