use anyhow::{anyhow, bail, ensure, Context, Result};
use chrono::{offset::Local, DateTime};
use futures_lite::stream::StreamExt;
use gtk::prelude::*;
use gtk::{gdk, glib, Align};
use gtk4_layer_shell::LayerShell;
use pulse::callbacks::ListResult;
use pulse::context::introspect::{SinkInfo, SourceInfo};
use pulse::volume::ChannelVolumes;
use relm4::prelude::*;
use rustix::system;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use swayipc_async::{Floating, NodeType};
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

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

#[derive(Debug, Clone)]
enum PulseKind {
    Sink,
    Source,
}

#[derive(Debug, Default, PartialEq)]
struct Pulse {
    muted: bool,
    volume: u32,
    icon: String,
}

impl Pulse {
    fn make(muted: bool, volume: ChannelVolumes, name: &str) -> Self {
        let volume = volume.avg().0 * 100 / 65536;
        let icon = match volume {
            0 => "muted",
            _ if muted => "muted",
            v if v <= 25 => "low",
            v if v <= 50 => "medium",
            v if v <= 100 => "high",
            _ => "high",
        };
        let icon = format!("{name}-volume-{icon}");
        Self {
            muted,
            volume,
            icon,
        }
    }
}

impl<T: Into<Pulse>> TryFrom<ListResult<T>> for Pulse {
    type Error = ();
    fn try_from(value: ListResult<T>) -> Result<Self, Self::Error> {
        let ListResult::Item(info) = value else {
            return Err(());
        };
        Ok(info.into())
    }
}

impl From<&'_ SinkInfo<'_>> for Pulse {
    fn from(value: &'_ SinkInfo) -> Self {
        Self::make(value.mute, value.volume, "audio")
    }
}

impl From<&'_ SourceInfo<'_>> for Pulse {
    fn from(value: &'_ SourceInfo) -> Self {
        Self::make(value.mute, value.volume, "mic")
    }
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
}

struct AppModel {
    monitor: gdk::Monitor,
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
}

#[relm4::component]
impl Component for AppModel {
    type Init = AppModel;
    type Input = AppInput;
    type Output = ();
    type CommandOutput = ();

    // TODO: prettify view
    view! {
        gtk::Window {
            init_layer_shell: (),
            set_monitor: &model.monitor,
            set_layer: gtk4_layer_shell::Layer::Top,
            auto_exclusive_zone_enable: (),
            set_anchor: (gtk4_layer_shell::Edge::Left, true),
            set_anchor: (gtk4_layer_shell::Edge::Right, true),
            set_anchor: (gtk4_layer_shell::Edge::Top, false),
            set_anchor: (gtk4_layer_shell::Edge::Bottom, true),
            add_css_class: "bar",

            gtk::CenterBox {

                #[wrap(Some)] set_start_widget = &gtk::Box {
                    set_halign: Align::Start,

                    #[name(workspace_number)] gtk::Button,
                    gtk::Button {
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
                        }
                    },
                },
            },
        }
    }

    // Initialize the UI.
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
            AppInput::Pulse(PulseKind::Sink),
            AppInput::Pulse(PulseKind::Source),
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
            AppInput::Pulse(PulseKind::Sink) => ui.sink.set_icon_name(Some(&state.sink.icon)),
            AppInput::Pulse(PulseKind::Source) => ui.source.set_icon_name(Some(&state.source.icon)),
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

async fn pulse_loop(tx: tokio::sync::oneshot::Sender<pulse::context::Context>) -> Result<()> {
    let mut proplist = pulse::proplist::Proplist::new().unwrap();
    proplist
        .set_str(pulse::proplist::properties::APPLICATION_NAME, "swaynyaad")
        .unwrap();

    let mut mainloop = pulse_tokio::TokioMain::new();
    let mut context =
        pulse::context::Context::new_with_proplist(&mainloop, "swaynyaad-context", &proplist)
            .context("pulse context")?;
    context
        .connect(None, pulse::context::FlagSet::NOFAIL, None)
        .context("pulse ctx connect")?;
    mainloop
        .wait_for_ready(&context)
        .await
        .map_err(|e| anyhow!("{e:?}, pulse wait ready"))?;
    ensure!(tx.send(context).is_ok());
    ensure!(0 == mainloop.run().await.0);
    Ok(())
}

async fn sound_updater(
    tx: mpsc::UnboundedSender<AppInput>,
    state: Arc<RwLock<AppState>>,
) -> Result<()> {
    let (ctx_tx, ctx_rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("pulse-event-loop".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let local_set = tokio::task::LocalSet::new();
            local_set.spawn_local(pulse_loop(ctx_tx));
            rt.block_on(local_set)
        })
        .context("pulse loop start")?;
    let mut context = ctx_rx.await?;

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<()>();
    use pulse::context::subscribe::InterestMaskSet;
    context.subscribe(InterestMaskSet::SINK | InterestMaskSet::SOURCE, |_| {});
    context.set_subscribe_callback(Some(Box::new(move |_, _, _| {
        event_tx.send(()).expect("internal send pulse");
    })));
    let intro = context.introspect();

    let (pulse_tx, mut pulse_rx) = mpsc::unbounded_channel::<(PulseKind, Pulse)>();

    tokio::spawn(async move {
        loop {
            intro.get_sink_info_by_name("@DEFAULT_SINK@", {
                let pulse_tx = pulse_tx.clone();
                move |info| {
                    if let Ok(info) = Pulse::try_from(info) {
                        pulse_tx.send((PulseKind::Sink, info)).unwrap();
                    }
                }
            });

            intro.get_source_info_by_name("@DEFAULT_SOURCE@", {
                let pulse_tx = pulse_tx.clone();
                move |info| {
                    if let Ok(info) = Pulse::try_from(info) {
                        pulse_tx.send((PulseKind::Source, info)).unwrap();
                    }
                }
            });

            event_rx.recv().await;
        }
    });

    while let Some((kind, pulse)) = pulse_rx.recv().await {
        let mut state = state.write().unwrap();
        let slot = match kind {
            PulseKind::Sink => &mut state.sink,
            PulseKind::Source => &mut state.source,
        };
        if *slot == pulse {
            continue;
        }
        // TODO notify!
        *slot = pulse;
        tx.send(AppInput::Pulse(kind)).context("send pulse")?;
    }

    Ok(())
}

async fn main_loop(app: gtk::Application) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let state = Arc::new(RwLock::new(AppState::default()));

    relm4::spawn(sway_state_listener(tx.clone(), Arc::clone(&state)));
    relm4::spawn(time_updater(tx.clone(), Arc::clone(&state)));
    relm4::spawn(sound_updater(tx.clone(), Arc::clone(&state)));

    let mut windows: HashMap<String, Controller<AppModel>> = HashMap::new();

    loop {
        let event = rx.recv().await.context("receive event")?;
        match event {
            AppInput::Outputs(new_outputs) => {
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

                    let mut controller = AppModel::builder()
                        .launch(AppModel {
                            monitor,
                            state: Arc::clone(&state),
                        })
                        .detach();
                    let window = controller.widget();
                    app.add_window(window);
                    window.set_visible(true);
                    controller.detach_runtime();

                    ensure!(
                        windows.insert(added.into(), controller).is_none(),
                        "nonexistent element exists"
                    );
                }
            }
            event => {
                // TODO skip redundant updates
                for controller in windows.values() {
                    controller.sender().emit(event.clone());
                }
            }
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
                if let Err(e) = main_loop(app).await {
                    eprintln!("{e:?}");
                    std::process::abort();
                }
            });
        });
    });

    app.run()
}
