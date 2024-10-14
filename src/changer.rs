use gtk::{gdk, prelude::*, Orientation};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use log::info;
use relm4::prelude::*;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

pub struct ChangerModel {
    monitor: gdk::Monitor,
    watcher: Arc<Notify>,
}

#[derive(Debug, Clone)]
pub enum ChangerInput {
    Hide,
    Show {
        icon: Arc<str>,
        name: Arc<str>,
        value: f64,
    },
}

impl ChangerModel {
    pub fn create(monitor: gdk::Monitor) -> Self {
        ChangerModel {
            monitor,
            watcher: Arc::new(Notify::new()),
        }
    }
}

#[relm4::component(pub)]
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
            set_margin: (Edge::Bottom, 48),
            add_css_class: "changer",
            set_visible: false,

            gtk::Box {
                set_orientation: Orientation::Vertical,
                set_spacing: 8,

                gtk::CenterBox {
                    #[wrap(Some)] #[name(icon)] set_start_widget = &gtk::Image,
                    #[wrap(Some)] #[name(name)] set_center_widget = &gtk::Label,
                    #[wrap(Some)] #[name(text)] set_end_widget = &gtk::Label,
                },
                #[name(value)] gtk::ProgressBar,
            },
        }
    }

    fn init(
        model: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        info!("Creating Changer for {:?}", model.monitor.connector());
        let widgets = view_output!();

        let notify = Arc::clone(&model.watcher);
        tokio::spawn(async move {
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
            ChangerInput::Show { name, icon, value } => {
                ui.window.set_visible(true);
                ui.name.set_text(&name);
                ui.icon.set_icon_name(Some(&icon));
                ui.text.set_text(&format!("{}", (value * 100.).round()));
                ui.value.set_fraction(value);
                self.watcher.notify_one();
            }
        }
    }
}
