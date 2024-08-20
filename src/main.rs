use anyhow::{bail, ensure, Context, Result};
use chrono::{offset::Local, DateTime};
use gtk::prelude::*;
use gtk::{gdk, glib, Align};
use gtk4_layer_shell::LayerShell;
use relm4::prelude::*;
use smol::stream::StreamExt;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use swayipc_async::{Floating, NodeType};

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

#[derive(Debug, Default)]
struct AppState {
    layout: XkbLayout,
    time: DateTime<Local>,
    workspaces_urgent: Vec<i32>,
    workspaces_existing: BTreeSet<i32>,
    screen_focused: Option<String>,
    screens: HashMap<String, Screen>,
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
    LoadAverage(String),
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
            set_layer: gtk4_layer_shell::Layer::Overlay,
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
                            set_spacing: 10,
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

                    #[name(workspaces_urgent)] gtk::Button,
                    #[name(load_average)] gtk::Label,
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

        sender.input_sender().emit(AppInput::Layout);
        sender.input_sender().emit(AppInput::Time);
        sender.input_sender().emit(AppInput::Workspaces);
        sender
            .input_sender()
            .emit(AppInput::LoadAverage("0".into()));

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
                    .set_icon_name(if state.workspaces_urgent.is_empty() {
                        "radio-symbolic"
                    } else {
                        "radio-checked-symbolic"
                    });

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
                ui.window_class.set_label(
                    &None
                        .or_else(|| focused.app_id.clone())
                        .or_else(|| serde_json::to_string(&focused.shell).ok())
                        .unwrap(),
                );
                ui.window_float.set_visible(focused.floating);
            }
            AppInput::LoadAverage(lavg) => ui.load_average.set_text(&lavg),
        }
    }
}

async fn time_updater(
    tx: smol::channel::Sender<AppInput>,
    state: Arc<RwLock<AppState>>,
) -> Result<()> {
    let mut timer = smol::Timer::interval(Duration::from_secs(1));

    loop {
        state.write().unwrap().time = Local::now();
        tx.send(AppInput::Time).await.context("send time")?;

        tx.send(AppInput::LoadAverage(
            std::fs::read_to_string("/proc/loadavg")
                .context("read lavg")?
                .split(' ')
                .next()
                .context("malformed lavg")?
                .to_owned(),
        ))
        .await
        .context("send lavg")?;

        if timer.next().await.is_none() {
            break;
        }
    }

    Ok(())
}

async fn sway_state_listener(
    tx: smol::channel::Sender<AppInput>,
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
    tx: &smol::channel::Sender<AppInput>,
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
    tx.send(AppInput::Layout).await.context("send layout")?;
    Ok(())
}

async fn sway_fetch_output(
    tx: &smol::channel::Sender<AppInput>,
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
        .await
        .context("send outputs")?;

    sway_fetch_workspace(tx, conn, state).await?;

    Ok(())
}

async fn sway_fetch_workspace(
    tx: &smol::channel::Sender<AppInput>,
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
    tx.send(AppInput::Workspaces)
        .await
        .context("send workspaces")?;

    Ok(())
}

async fn main_loop(app: gtk::Application) -> Result<()> {
    let (tx, rx) = smol::channel::unbounded();
    let state = Arc::new(RwLock::new(AppState::default()));

    relm4::spawn_local(sway_state_listener(tx.clone(), Arc::clone(&state)));
    relm4::spawn_local(time_updater(tx.clone(), Arc::clone(&state)));

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
                    // XXX: Blocking call??
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
