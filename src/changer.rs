use gtk::{gdk, prelude::*, Align, IconSize};
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
        model: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        info!("Creating Changer for {:?}", model.monitor.connector());
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
            ChangerInput::Show { name, icon, value } => {
                ui.window.set_visible(true);
                ui.name.set_text(&name);
                ui.icon.set_icon_name(Some(&icon));
                ui.value.set_fraction(value);
                self.watcher.notify_one();
            }
        }
    }
}
