use crate::bar::{AppInput, AppModel};
use crate::listeners;
use crate::state::AppState;
use eyre::{ensure, Context, OptionExt, Result};
use gtk::{gdk, prelude::*};
use log::{debug, info, trace, warn};
use relm4::prelude::*;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Source};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

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

    // XXX should it be cached?
    let path = format!("/usr/share/sounds/freedesktop/stereo/{name}.oga");
    let file = std::io::BufReader::new(std::fs::File::open(path).context("open audio file")?);
    let source = Decoder::new(file).context("decode audio")?;
    stream_handle
        .play_raw(source.convert_samples())
        .context("play audio")?;

    Ok(())
}

fn adjust_windows(
    state: Arc<RwLock<AppState>>,
    windows: &mut HashMap<String, Controller<AppModel>>,
    new_outputs: HashSet<String>,
) -> Result<()> {
    // XXX is it really needed to `Drop` bar windows instead of just hiding them?
    // Check behavior of monitor used for layer shell vanishing
    windows.retain(|output, _| new_outputs.contains(output));

    let monitors = gdk::Display::default()
        .ok_or_eyre("Failed to get default display")?
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
            .ok_or_eyre("unknown monitor");
        let Ok(monitor) = monitor else {
            warn!("GDK and Sway monitor mismatch! {added} exists, but not for GDK");
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
    Ok(())
}

fn forward_event(event: AppInput, windows: &HashMap<String, Controller<AppModel>>) -> Result<()> {
    // XXX is it possible to use broadcast channels here?
    for controller in windows.values() {
        controller.sender().emit(event.clone());
    }
    Ok(())
}

pub async fn main_loop() -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let state = Arc::new(RwLock::new(AppState::default()));

    listeners::start(tx, Arc::clone(&state));

    let mut windows: HashMap<String, Controller<AppModel>> = HashMap::new();

    let (_stream, stream_handle) = OutputStream::try_default().context("create output stream")?;

    info!("Ready dispatching events");

    loop {
        let event = rx.recv().await.ok_or_eyre("receive event")?;
        debug!("Received {event:?}");
        trace!("Current state is {:#?}", state.read().unwrap());

        let AppInput::Outputs(new_outputs) = event else {
            play_sound(&stream_handle, &state.read().unwrap(), &event)?;
            forward_event(event, &windows)?;
            continue;
        };

        adjust_windows(Arc::clone(&state), &mut windows, new_outputs)?;
    }
}
