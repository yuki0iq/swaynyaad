use alsa::mixer::{Mixer, Selem, SelemChannelId};
use alsa::poll::{pollfd, Descriptors};
use anyhow::{bail, ensure, Context, Result};
use chrono::{offset::Local, DateTime};
use futures_lite::stream::StreamExt;
use gtk::prelude::*;
use gtk::{gdk, glib, Align, IconSize};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use kira::manager::{AudioManager, AudioManagerSettings, DefaultBackend};
use kira::sound::static_sound::StaticSoundData;
use relm4::prelude::*;
use rustix::system;
use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use swayipc_async::{Floating, NodeType};
use tokio::fs::File;
use tokio::io::unix::AsyncFd;
use tokio::io::{AsyncBufReadExt, BufReader, Interest};
use tokio::sync::{mpsc, Notify};
use upower_dbus::{DeviceProxy, UPowerProxy};

#[derive(Debug, Default, Clone, PartialEq)]
struct XkbLayout {
    name: String,
    description: String,
}

#[derive(Debug, Default)]
struct Node {
    shell: String,
    app_id: Option<String>,
    floating: bool,
}

#[derive(Debug, Default)]
struct Screen {
    workspace: Option<String>,
    focused: Option<Node>,
}

#[derive(Debug, Clone, Copy)]
enum PulseKind {
    Sink,
    Source,
}

#[derive(Debug, Default, PartialEq)]
struct Pulse {
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
struct Power {
    present: bool,
    icon: String,
}

#[derive(Debug, Default)]
struct AppState {
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

#[derive(Default, Debug, Clone)]
struct ChangerState {
    icon: String,
    name: String,
    value: f64,
}

struct ChangerModel {
    monitor: gdk::Monitor,
    watcher: Arc<Notify>,
}

#[derive(Debug, Clone)]
enum ChangerInput {
    Hide,
    Show(ChangerState),
}

impl ChangerModel {
    fn create(monitor: gdk::Monitor) -> Self {
        ChangerModel {
            monitor,
            watcher: Arc::new(Notify::new()),
        }
    }
}

#[relm4::component]
impl Component for ChangerModel {
    type Init = ChangerModel;
    type Input = ChangerInput;
    type Output = ();
    type CommandOutput = ();

    view! {
        #[name(window)] gtk::Window {
            init_layer_shell: (),
            set_monitor: &model.monitor,
            set_layer: Layer::Overlay,
            set_anchor: (Edge::Bottom, true),
            set_margin: (Edge::Bottom, 40),
            add_css_class: "changer",
            set_visible: false,

            gtk::Grid {
                set_column_spacing: 16,
                set_row_spacing: 8,
                set_halign: Align::Center,
                set_valign: Align::Center,

                attach[0, 0, 1, 2]: icon = &gtk::Image {
                    set_icon_size: IconSize::Large,
                },
                attach[1, 0, 1, 1]: name = &gtk::Label,
                attach[1, 1, 1, 1]: value = &gtk::ProgressBar,
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = params;

        let widgets = view_output!();

        let notify = Arc::clone(&model.watcher);
        relm4::spawn(async move {
            loop {
                let event = tokio::time::timeout(Duration::from_secs(1), notify.notified()).await;
                if event.is_err() {
                    sender.input(ChangerInput::Hide);
                }
            }
        });

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        ui: &mut Self::Widgets,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            ChangerInput::Hide => ui.window.set_visible(false),
            ChangerInput::Show(state) => {
                ui.window.set_visible(true);
                ui.name.set_text(&state.name);
                ui.icon.set_icon_name(Some(&state.icon));
                ui.value.set_fraction(state.value);
                self.watcher.notify_one();
            }
        }
    }
}

struct AppModel {
    monitor: gdk::Monitor,
    changer: Controller<ChangerModel>,
    state: Arc<RwLock<AppState>>,
}

#[derive(Debug, Clone)]
enum AppInput {
    Outputs(HashSet<String>),
    Layout,
    Time,
    Workspaces,
    Sysinfo,
    Pulse(PulseKind),
    Power,
    PowerPlugged,
    PowerUnplugged,
}

impl AppModel {
    fn create(state: Arc<RwLock<AppState>>, monitor: gdk::Monitor) -> Self {
        Self {
            changer: ChangerModel::builder()
                .launch(ChangerModel::create(monitor.clone()))
                .detach(),

            monitor,
            state,
        }
    }
}

#[relm4::component]
impl Component for AppModel {
    type Init = AppModel;
    type Input = AppInput;
    type Output = ();
    type CommandOutput = ();

    view! {
        gtk::Window {
            init_layer_shell: (),
            set_monitor: &model.monitor,
            set_layer: Layer::Top,
            auto_exclusive_zone_enable: (),
            set_anchor: (Edge::Left, true),
            set_anchor: (Edge::Right, true),
            set_anchor: (Edge::Top, true),
            set_anchor: (Edge::Bottom, false),
            add_css_class: "bar",
            set_visible: true,

            gtk::CenterBox {

                #[wrap(Some)] set_start_widget = &gtk::Box {
                    set_halign: Align::Start,

                    #[name(workspace_number)] gtk::Button,
                    #[name(window)] gtk::Button {
                        #[wrap(Some)] set_child = &gtk::Box {
                            set_spacing: 8,
                            #[name(window_class)] gtk::Label,
                            #[name(window_float)] gtk::Image {
                                set_icon_name: Some("object-move-symbolic"),
                                set_visible: false
                            },
                        },
                    },
                },

                #[wrap(Some)] set_center_widget = &gtk::Box {
                    set_halign: Align::Center,

                    #[name(date)] gtk::Button,
                    #[name(layout)] gtk::Button,
                },

                #[wrap(Some)] set_end_widget = &gtk::Box {
                    set_halign: Align::End,

                    gtk::Button {
                        #[wrap(Some)] set_child = &gtk::Box {
                            set_spacing: 8,
                            #[name(workspaces_urgent)] gtk::Image {
                                set_icon_name: Some("xfce-wm-stick"),
                            },
                            #[name(sink)] gtk::Image,
                            #[name(source)] gtk::Image,
                            #[name(load_average)] gtk::Label,
                            #[name(used_ram)] gtk::Label,
                            #[name(power)] gtk::Image,
                        }
                    },
                },
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = params;

        let widgets = view_output!();

        for event in [
            AppInput::Layout,
            AppInput::Time,
            AppInput::Workspaces,
            AppInput::Sysinfo,
            AppInput::Pulse(PulseKind::Source),
            AppInput::Pulse(PulseKind::Sink),
            AppInput::Power,
        ] {
            sender.input_sender().emit(event);
        }

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        ui: &mut Self::Widgets,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        let state = self.state.read().unwrap();
        match message {
            AppInput::Outputs(_) => {}
            AppInput::Layout => ui.layout.set_label(&state.layout.name),
            AppInput::Time => {
                ui.date
                    .set_label(&state.time.format("%a %b %-d \t %T").to_string());
            }
            AppInput::Workspaces => {
                ui.workspaces_urgent
                    .set_visible(!state.workspaces_urgent.is_empty());

                let mon = self.monitor.connector();
                let mon = mon.as_deref().unwrap();
                let Some(screen) = state.screens.get(mon) else {
                    return;
                };
                ui.workspace_number
                    .set_label(screen.workspace.as_ref().unwrap());
                ui.window.set_visible(screen.focused.is_some());

                let Some(focused) = &screen.focused else {
                    return;
                };
                ui.window_class
                    .set_label(focused.app_id.as_ref().unwrap_or(&focused.shell));
                ui.window_float.set_visible(focused.floating);
            }
            AppInput::Sysinfo => {
                ui.load_average
                    .set_text(&format!("{:0.2}", state.load_average));
                ui.used_ram.set_text(&format!("{:0.2}", state.memory_usage));
            }
            AppInput::Pulse(kind) => {
                let name = match kind {
                    PulseKind::Sink => "Speakers",
                    PulseKind::Source => "Microphone",
                };
                let pulse = match kind {
                    PulseKind::Sink => &state.sink,
                    PulseKind::Source => &state.source,
                };
                let ui_icon = match kind {
                    PulseKind::Sink => &ui.sink,
                    PulseKind::Source => &ui.source,
                };

                ui_icon.set_icon_name(Some(&pulse.icon));

                self.changer.sender().emit(ChangerInput::Show(ChangerState {
                    icon: pulse.icon.clone(),
                    name: name.into(),
                    value: pulse.volume as f64 / 100.,
                }));
            }
            AppInput::Power => {
                ui.power.set_visible(state.power.present);
                ui.power.set_icon_name(Some(&state.power.icon));
            }
            AppInput::PowerPlugged => {
                self.changer.sender().emit(ChangerInput::Show(ChangerState {
                    icon: "ac-adapter-symbolic".into(),
                    name: "AC plugged".into(),
                    value: 100.,
                }))
            }
            AppInput::PowerUnplugged => {
                self.changer.sender().emit(ChangerInput::Show(ChangerState {
                    icon: "battery-symbolic".into(),
                    name: "On battery".into(),
                    value: 0.,
                }))
            }
        }
    }
}

async fn time_updater(
    tx: mpsc::UnboundedSender<AppInput>,
    state: Arc<RwLock<AppState>>,
) -> Result<()> {
    let mut timer = tokio::time::interval(Duration::from_secs(1));

    loop {
        state.write().unwrap().time = Local::now();
        tx.send(AppInput::Time).context("send time")?;

        {
            let sysinfo = system::sysinfo();

            let meminfo = File::open("/proc/meminfo").await.context("read meminfo")?;
            let mut meminfo = BufReader::new(meminfo).lines();
            let mut total_ram: usize = 1;
            let mut available_ram: usize = 0;
            let mut count_fields = 2;
            while let Some(line) = meminfo.next_line().await.context("line meminfo")? {
                let entries = line.split_whitespace().collect::<Vec<_>>();
                match entries[..] {
                    [name, value, _unit] => match name {
                        "MemTotal:" => {
                            total_ram = value.parse().context("bad total_ram")?;
                            count_fields -= 1;
                        }
                        "MemAvailable:" => {
                            available_ram = value.parse().context("bad available_ram")?;
                            count_fields -= 1;
                        }
                        _ => {}
                    },
                    [_name, _value] => {}
                    _ => bail!("/proc/meminfo has unexpected format"),
                }

                if count_fields == 0 {
                    break;
                }
            }

            let load_average = sysinfo.loads[0] as f64 / 65536.;
            let memory_usage = 1. - available_ram as f64 / total_ram as f64;

            let mut state = state.write().unwrap();
            state.load_average = load_average;
            state.memory_usage = memory_usage;
            tx.send(AppInput::Sysinfo).context("send sysinfo")?;
        }

        let _ = timer.tick().await;
    }
}

async fn sway_state_listener(
    tx: mpsc::UnboundedSender<AppInput>,
    state: Arc<RwLock<AppState>>,
) -> Result<()> {
    use swayipc_async::{Connection, Event, EventType};

    let mut conn = Connection::new().await.context("initial connection")?;
    let mut stream = Connection::new()
        .await
        .context("event connection")?
        .subscribe([
            EventType::Input,
            EventType::Output,
            EventType::Workspace,
            EventType::Window,
        ])
        .await
        .context("subscribe to events")?;

    sway_fetch_output(&tx, &mut conn, Arc::clone(&state))
        .await
        .context("init output")?;
    sway_fetch_input(&tx, &mut conn, Arc::clone(&state))
        .await
        .context("init input")?;

    while let Some(event) = stream.next().await {
        let Ok(event) = event else { continue };
        match event {
            Event::Input(_) => sway_fetch_input(&tx, &mut conn, Arc::clone(&state))
                .await
                .context("fetch input")?,
            Event::Output(_) => sway_fetch_output(&tx, &mut conn, Arc::clone(&state))
                .await
                .context("fetch output")?,
            Event::Window(_) | Event::Workspace(_) => {
                sway_fetch_workspace(&tx, &mut conn, Arc::clone(&state))
                    .await
                    .context("fetch workspace")?
            }
            _ => bail!("Unexpected event"),
        }
    }

    Ok(())
}

async fn sway_fetch_input(
    tx: &mpsc::UnboundedSender<AppInput>,
    conn: &mut swayipc_async::Connection,
    state: Arc<RwLock<AppState>>,
) -> Result<()> {
    let inputs = conn.get_inputs().await.context("get inputs")?;

    let layout_name = inputs
        .iter()
        .find_map(|input| input.xkb_active_layout_name.as_ref());

    state.write().unwrap().layout = XkbLayout {
        name: layout_name
            .map(|layout| layout[..2].to_ascii_lowercase())
            .unwrap_or_else(|| "xx".into()),
        description: layout_name
            .cloned()
            .unwrap_or_else(|| "Unknown layout".into()),
    };
    tx.send(AppInput::Layout).context("send layout")?;
    Ok(())
}

async fn sway_fetch_output(
    tx: &mpsc::UnboundedSender<AppInput>,
    conn: &mut swayipc_async::Connection,
    state: Arc<RwLock<AppState>>,
) -> Result<()> {
    let outputs = conn
        .get_outputs()
        .await
        .context("get outputs")?
        .into_iter()
        .map(|out| out.name)
        .collect::<HashSet<_>>();

    tx.send(AppInput::Outputs(outputs))
        .context("send outputs")?;

    sway_fetch_workspace(tx, conn, state).await?;

    Ok(())
}

async fn sway_fetch_workspace(
    tx: &mpsc::UnboundedSender<AppInput>,
    conn: &mut swayipc_async::Connection,
    state: Arc<RwLock<AppState>>,
) -> Result<()> {
    let workspaces = conn.get_workspaces().await.context("get workspaces")?;
    let workspaces_existing = workspaces.iter().map(|ws| ws.num).collect::<BTreeSet<_>>();
    let workspaces_urgent = workspaces
        .iter()
        .filter(|ws| ws.urgent)
        .map(|ws| ws.num)
        .collect::<Vec<_>>();

    let outputs = conn.get_outputs().await.context("get outputs")?;
    let screen_focused = outputs
        .iter()
        .find(|output| output.focused)
        .map(|output| output.name.clone());

    let tree = conn.get_tree().await.context("get tree")?;

    let mut screens = HashMap::new();
    for output in outputs {
        // This is O(total_nodes), and not O(workspaces)
        let workspace = tree.find_as_ref(|node| {
            node.node_type == NodeType::Workspace && node.name == output.current_workspace
        });
        let focused = workspace.and_then(|ws| {
            ws.find_focused_as_ref(|node| {
                matches!(node.node_type, NodeType::FloatingCon | NodeType::Con)
                    && node.nodes.is_empty()
            })
        });
        screens.insert(
            output.name,
            Screen {
                workspace: output.current_workspace,
                focused: focused.map(|node| Node {
                    shell: serde_json::to_string(&node.shell).unwrap(),
                    floating: matches!(
                        node.floating,
                        Some(Floating::AutoOn) | Some(Floating::UserOn)
                    ),
                    app_id: node.app_id.clone().or_else(|| {
                        Some(format!(
                            "{} [X11]",
                            node.window_properties.as_ref()?.class.as_ref()?
                        ))
                    }),
                }),
            },
        );
    }

    {
        let mut state = state.write().unwrap();
        state.workspaces_urgent = workspaces_urgent;
        state.workspaces_existing = workspaces_existing;
        state.screen_focused = screen_focused;
        state.screens = screens;
    }
    tx.send(AppInput::Workspaces).context("send workspaces")?;

    Ok(())
}

async fn alsa_loop(pulse_tx: mpsc::UnboundedSender<(PulseKind, Pulse)>) -> Result<()> {
    let mixer = Mixer::new("default", false).context("alsa mixer create")?;

    let mut fds: Vec<pollfd> = vec![];
    loop {
        mixer.handle_events().context("alsa mixer handle events")?;

        for elem in mixer.iter() {
            let Some(selem) = Selem::new(elem) else {
                continue;
            };

            let kind = match selem.get_id().get_name() {
                Ok("Master") => PulseKind::Sink,
                Ok("Capture") => PulseKind::Source,
                _ => continue,
            };

            pulse_tx
                .send((kind, Pulse::make(selem, kind)))
                .ok()
                .context("send alsa")?;
        }

        let count = Descriptors::count(&mixer);
        fds.resize_with(count, || pollfd {
            fd: 0,
            events: 0,
            revents: 0,
        });
        Descriptors::fill(&mixer, &mut fds).context("fill descriptors")?;

        let mut futs = Vec::with_capacity(count);
        for pfd in &fds {
            let fd = pfd.fd;
            futs.push(tokio::spawn(async move {
                let interest = Interest::ERROR | Interest::READABLE;
                let afd = AsyncFd::with_interest(fd, interest).unwrap();
                let res = afd.ready(interest).await;
                res.map(|mut guard| guard.clear_ready())
            }));
        }
        let _ = futures::future::select_all(futs).await;
    }
}

async fn sound_updater(
    tx: mpsc::UnboundedSender<AppInput>,
    state: Arc<RwLock<AppState>>,
) -> Result<()> {
    let (pulse_tx, mut pulse_rx) = mpsc::unbounded_channel();
    relm4::spawn(alsa_loop(pulse_tx));

    while let Some((kind, pulse)) = pulse_rx.recv().await {
        let mut state = state.write().unwrap();
        let slot = match kind {
            PulseKind::Sink => &mut state.sink,
            PulseKind::Source => &mut state.source,
        };
        if *slot == pulse {
            continue;
        }
        *slot = pulse;
        tx.send(AppInput::Pulse(kind)).context("send pulse")?;
    }

    Ok(())
}

async fn upower_show(
    tx: mpsc::UnboundedSender<AppInput>,
    state: Arc<RwLock<AppState>>,
    device: DeviceProxy<'_>,
) -> Result<()> {
    let mut changed = device.receive_is_present_changed().await;
    while let Some(value) = changed.next().await {
        state.write().unwrap().power.present = value.get().await?;
        tx.send(AppInput::Power).context("upower present")?;
    }
    Ok(())
}

async fn upower_icon(
    tx: mpsc::UnboundedSender<AppInput>,
    state: Arc<RwLock<AppState>>,
    device: DeviceProxy<'_>,
) -> Result<()> {
    let mut changed = device.receive_icon_name_changed().await;
    while let Some(value) = changed.next().await {
        state.write().unwrap().power.icon = value.get().await?;
        tx.send(AppInput::Power).context("upower icon")?;
    }
    Ok(())
}

async fn upower_plug(tx: mpsc::UnboundedSender<AppInput>, upower: UPowerProxy<'_>) -> Result<()> {
    let mut changed = upower.receive_on_battery_changed().await;
    while let Some(value) = changed.next().await {
        tx.send(if value.get().await? {
            AppInput::PowerPlugged
        } else {
            AppInput::PowerUnplugged
        })
        .context("upower plug")?;
    }
    Ok(())
}

async fn upower_listener(
    tx: mpsc::UnboundedSender<AppInput>,
    state: Arc<RwLock<AppState>>,
    conn: zbus::Connection,
) -> Result<()> {
    let upower = UPowerProxy::new(&conn).await.context("bind to upower")?;
    let device = upower.get_display_device().await?;

    let present = device.is_present().await?;
    let icon = device.icon_name().await?;
    state.write().unwrap().power = Power { present, icon };
    tx.send(AppInput::Power).context("upower init")?;

    tokio::spawn(upower_show(tx.clone(), Arc::clone(&state), device.clone()));
    tokio::spawn(upower_icon(tx.clone(), Arc::clone(&state), device.clone()));
    tokio::spawn(upower_plug(tx.clone(), upower.clone()));

    Ok(())
}

async fn zbus_listener(
    tx: mpsc::UnboundedSender<AppInput>,
    state: Arc<RwLock<AppState>>,
) -> Result<()> {
    let conn = zbus::Connection::system()
        .await
        .context("connect to system bus")?;

    tokio::spawn(upower_listener(
        tx.clone(),
        Arc::clone(&state),
        conn.clone(),
    ));

    Ok(())
}

fn play_sound(
    manager: &mut AudioManager,
    cache: &mut HashMap<&'static str, StaticSoundData>,
    event: &AppInput,
) -> Result<()> {
    let name = match event {
        AppInput::Pulse(_) => "audio-volume-change",

        _ => return Ok(()),
    };

    let sound_data = match cache.entry(name) {
        Entry::Vacant(vacant) => vacant
            .insert(
                StaticSoundData::from_file(format!(
                    "/usr/share/sounds/freedesktop/stereo/{name}.oga"
                ))
                .context("open sound")?,
            )
            .clone(),
        Entry::Occupied(occupied) => occupied.get().clone(),
    };

    manager.play(sound_data).context("play data")?;

    Ok(())
}

async fn main_loop() -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let state = Arc::new(RwLock::new(AppState::default()));

    relm4::spawn(sway_state_listener(tx.clone(), Arc::clone(&state)));
    relm4::spawn(time_updater(tx.clone(), Arc::clone(&state)));
    relm4::spawn(sound_updater(tx.clone(), Arc::clone(&state)));
    relm4::spawn(zbus_listener(tx.clone(), Arc::clone(&state)));

    let mut windows: HashMap<String, Controller<AppModel>> = HashMap::new();

    let mut manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
        .context("create audio manager")?;
    let mut sound_cache = HashMap::new();

    loop {
        let event = rx.recv().await.context("receive event")?;
        let AppInput::Outputs(new_outputs) = event else {
            play_sound(&mut manager, &mut sound_cache, &event).context("play sound")?;

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
                .context("unknown monitor")?
                .clone();

            let controller = AppModel::builder()
                .launch(AppModel::create(Arc::clone(&state), monitor))
                .detach();

            ensure!(
                windows.insert(added.into(), controller).is_none(),
                "nonexistent element exists"
            );
        }
    }
}

fn main() -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id("sylfn.swaynyaad.Bar")
        .build();

    let start = std::sync::Once::new();
    app.connect_activate(move |app| {
        let app = app.to_owned();
        start.call_once(move || {
            std::mem::forget(app.hold());
            relm4::set_global_css_from_file("/home/yuki/kek/swaynyaad/src/style.css").unwrap();
            relm4::spawn_local(async move {
                if let Err(e) = main_loop().await {
                    eprintln!("{e:?}");
                    std::process::abort();
                }
            });
        });
    });

    app.run()
}
