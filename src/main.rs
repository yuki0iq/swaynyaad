use alsa::mixer::{Selem, SelemChannelId};
use anyhow::{ensure, Context, Result};
use chrono::{offset::Local, DateTime};
use gtk::prelude::*;
use gtk::{gdk, glib};
use log::{debug, error, info, trace, warn};
use relm4::prelude::*;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Source};
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

mod bar;
mod changer;
mod critical;
mod listeners;

use self::bar::{AppInput, AppModel};

#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct XkbLayout {
    name: String,
    description: String,
}

#[derive(Debug, Default)]
pub(crate) struct Node {
    shell: String,
    app_id: Option<String>,
    floating: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Screen {
    workspace: Option<String>,
    focused: Option<Node>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PulseKind {
    Sink,
    Source,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct Pulse {
    muted: bool,
    volume: i64,
    icon: String,
}

impl Pulse {
    fn parse(selem: Selem, kind: PulseKind) -> (i64, bool) {
        let has_volume = match kind {
            PulseKind::Sink => selem.has_playback_volume(),
            PulseKind::Source => selem.has_capture_volume(),
        };
        if !has_volume {
            // This device is probably nonexistent. Returning anything "normal" is okay
            return (0, true);
        }

        let (volume_low, volume_high) = match kind {
            PulseKind::Sink => selem.get_playback_volume_range(),
            PulseKind::Source => selem.get_capture_volume_range(),
        };

        let mut globally_muted = match kind {
            PulseKind::Sink => selem.has_playback_switch(),
            PulseKind::Source => selem.has_capture_switch(),
        };

        let mut channel_count = 0;
        let mut acc_volume = 0;
        for scid in SelemChannelId::all() {
            let Ok(cur_volume) = (match kind {
                PulseKind::Sink => selem.get_playback_volume(*scid),
                PulseKind::Source => selem.get_capture_volume(*scid),
            }) else {
                continue;
            };

            let cur_muted = match kind {
                PulseKind::Sink => selem.get_playback_switch(*scid),
                PulseKind::Source => selem.get_capture_switch(*scid),
            } == Ok(0);

            globally_muted = globally_muted && cur_muted;
            channel_count += 1;
            if !cur_muted {
                acc_volume += cur_volume - volume_low;
            }
        }

        let volume = 100 * acc_volume / (volume_high - volume_low) / channel_count;
        (volume, globally_muted)
    }

    fn make(selem: Selem, kind: PulseKind) -> Self {
        let (volume, muted) = Self::parse(selem, kind);

        let icon = format!(
            "{}-volume-{}",
            match kind {
                PulseKind::Sink => "audio",
                PulseKind::Source => "mic",
            },
            match volume {
                0 => "muted",
                _ if muted => "muted",
                v if v <= 25 => "low",
                v if v <= 50 => "medium",
                v if v <= 100 => "high",
                _ => "high",
            }
        );

        Self {
            icon,
            muted,
            volume,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct Power {
    present: bool,
    charging: bool,
    level: f64,
    icon: String,
}

impl Power {
    fn is_critical(&self) -> bool {
        self.present && !self.charging && self.level < 10.
    }
}

#[derive(Debug, Default)]
pub(crate) struct AppState {
    layout: XkbLayout,
    time: DateTime<Local>,
    workspaces_urgent: Vec<i32>,
    workspaces_existing: BTreeSet<i32>,
    screen_focused: Option<String>,
    screens: HashMap<String, Screen>,
    load_average: f64,
    memory_usage: f64,
    sink: Pulse,
    source: Pulse,
    power: Power,
}

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
