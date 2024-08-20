use anyhow::{bail, Context, Result};
use chrono::{offset::Local, DateTime};
use gtk::prelude::*;
use gtk::{gdk, glib};
use gtk4_layer_shell::LayerShell;
use relm4::prelude::*;
use smol::stream::StreamExt;
use std::time::Duration;

#[derive(Debug, Default, PartialEq)]
struct XkbLayout {
    name: String,
    description: String,
}

#[tracker::track]
#[derive(Debug, Default)]
struct AppModel {
    layout: XkbLayout,
    time: DateTime<Local>,
}

#[derive(Debug)]
enum AppInput {
    Layout(XkbLayout),
    Time(DateTime<Local>),
}

#[relm4::component]
impl SimpleComponent for AppModel {
    type Init = gdk::Monitor;
    type Input = AppInput;
    type Output = ();

    // TODO: prettify view
    view! {
        // TODO: multi-monitor
        gtk::Window {
            init_layer_shell: (),
            set_monitor: &monitor,
            set_layer: gtk4_layer_shell::Layer::Overlay,
            auto_exclusive_zone_enable: (),
            set_anchor: (gtk4_layer_shell::Edge::Left, true),
            set_anchor: (gtk4_layer_shell::Edge::Right, true),
            set_anchor: (gtk4_layer_shell::Edge::Top, false),
            set_anchor: (gtk4_layer_shell::Edge::Bottom, true),

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,
                gtk::Label {
                    #[track = "model.changed(AppModel::layout())"]
                    set_text: &model.get_layout().name,
                    set_tooltip: &model.get_layout().description,
                },
                gtk::Label {
                    #[track = "model.changed(AppModel::time())"]
                    set_text: &model.get_time().format("%b %-d, %a").to_string(),
                },
                gtk::Label {
                    #[track = "model.changed(AppModel::time())"]
                    set_text: &model.get_time().format("%T").to_string(),
                },
            }
        }
    }

    // Initialize the UI.
    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let monitor = params;

        let model = AppModel::default();

        let input_sender = sender.input_sender();
        relm4::spawn(sway_state_listener(input_sender.clone()));
        relm4::spawn(time_updater(input_sender.clone()));

        // Insert the macro code generation here
        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
        // reset tracker value of the model
        self.reset();

        match message {
            AppInput::Layout(layout) => self.set_layout(layout),
            AppInput::Time(time) => self.set_time(time),
        }
    }
}

async fn time_updater(tx: relm4::Sender<AppInput>) -> Result<()> {
    let mut timer = smol::Timer::interval(Duration::from_secs(1));

    // Do-while loop
    while {
        tx.emit(AppInput::Time(Local::now()));
        timer.next().await.is_some()
    } {}

    Ok(())
}

async fn sway_state_listener(tx: relm4::Sender<AppInput>) -> Result<()> {
    use swayipc_async::{Connection, Event, EventType};

    let mut conn = Connection::new().await.context("initial connection")?;
    let mut stream = Connection::new()
        .await
        .context("event connection")?
        .subscribe([EventType::Input])
        .await
        .context("subscribe to events")?;

    // Initial state
    sway_fetch_input(&tx, &mut conn)
        .await
        .context("init input")?;

    while let Some(event) = stream.next().await {
        let Ok(event) = event else { continue };
        match event {
            Event::Input(_) => sway_fetch_input(&tx, &mut conn)
                .await
                .context("fetch input")?,
            _ => bail!("Unexpected event"),
        }
    }

    Ok(())
}

async fn sway_fetch_input(
    tx: &relm4::Sender<AppInput>,
    conn: &mut swayipc_async::Connection,
) -> Result<()> {
    let inputs = conn.get_inputs().await.context("get inputs")?;

    let layout_name = inputs
        .iter()
        .find_map(|input| input.xkb_active_layout_name.as_ref());

    tx.emit(AppInput::Layout(XkbLayout {
        name: layout_name
            .map(|layout| layout[..2].to_ascii_lowercase())
            .unwrap_or_else(|| "xx".into()),
        description: layout_name
            .cloned()
            .unwrap_or_else(|| "Unknown layout".into()),
    }));
    Ok(())
}

async fn main_loop(app: gtk::Application) -> Result<()> {
    let output = "VGA-1".to_owned();
    let monitor = gdk::Display::default()
        .context("Failed to get default display")?
        .monitors()
        .into_iter()
        .take_while(Result::is_ok)
        .flatten()
        .flat_map(|res| res.downcast::<gdk::Monitor>())
        .find(|monitor| monitor.connector().as_deref() == Some(&output))
        .context("Failed to find monitor for output")?;

    let mut controller = AppModel::builder().launch(monitor).detach();
    let window = controller.widget();
    app.add_window(window);
    window.set_visible(true);
    controller.detach_runtime();

    Ok(())
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
