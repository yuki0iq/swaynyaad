use crate::changer::{ChangerInput, ChangerModel};
use crate::critical::{CriticalInput, CriticalModel};
use crate::state::{AppState, PulseKind};
use gtk::{gdk, prelude::*, Align};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use heck::ToTitleCase;
use log::info;
use relm4::prelude::*;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};

pub(crate) struct AppModel {
    monitor: gdk::Monitor,
    changer: Controller<ChangerModel>,
    critical: Controller<CriticalModel>,
    state: Arc<RwLock<AppState>>,
}

#[derive(Debug, Clone)]
pub(crate) enum AppInput {
    Outputs(HashSet<String>),
    Layout,
    Time,
    Workspaces,
    Sysinfo,
    Pulse(PulseKind),
    Power,
    PowerChanged,
}

impl AppModel {
    pub fn create(state: Arc<RwLock<AppState>>, monitor: gdk::Monitor) -> Self {
        Self {
            changer: ChangerModel::builder()
                .launch(ChangerModel::create(monitor.clone()))
                .detach(),
            critical: CriticalModel::builder()
                .launch(CriticalModel {
                    monitor: monitor.clone(),
                })
                .detach(),

            monitor,
            state,
        }
    }
}

#[relm4::component(pub)]
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
                    set_spacing: 8,

                    gtk::MenuButton {
                        #[wrap(Some)] #[name(workspace_number)] set_child = &gtk::Label,
                    },
                    #[name(window)] gtk::MenuButton {
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
                    set_spacing: 8,

                    gtk::MenuButton {
                        #[wrap(Some)] #[name(date)] set_child = &gtk::Label,
                        #[wrap(Some)] set_popover = &gtk::Popover {
                            // TODO styles and date.
                            #[wrap(Some)] set_child = &gtk::Calendar,
                        },
                    },
                    gtk::MenuButton {
                        #[wrap(Some)] #[name(layout)] set_child = &gtk::Label,
                    },
                },

                #[wrap(Some)] set_end_widget = &gtk::Box {
                    set_halign: Align::End,

                    gtk::MenuButton {
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
                        },

                        // TODO populate "system" menu
                        #[wrap(Some)] set_popover = &gtk::Popover {
                            #[wrap(Some)] set_child = &gtk::Label {
                                set_text: "NYAAA hello world",
                            },
                        },
                    },
                },
            },
        }
    }

    fn init(
        model: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        info!("Creating App for {:?}", model.monitor.connector());
        let widgets = view_output!();

        for event in [
            AppInput::Layout,
            AppInput::Time,
            AppInput::Workspaces,
            AppInput::Sysinfo,
            AppInput::Pulse(PulseKind::Source),
            AppInput::Pulse(PulseKind::Sink),
            AppInput::Power,
            AppInput::PowerChanged,
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

                self.changer.sender().emit(ChangerInput::Show {
                    icon: pulse.icon.clone().into(),
                    name: name.into(),
                    value: pulse.volume as f64 / 100.,
                });
            }
            AppInput::Power => {
                ui.power.set_visible(state.power.present);
                ui.power.set_icon_name(Some(&state.power.icon));

                self.critical.sender().emit(if state.power.is_critical() {
                    CriticalInput::Show("Connect power NOW!".into())
                } else {
                    CriticalInput::Hide
                });
            }
            AppInput::PowerChanged => {
                self.changer.sender().emit(ChangerInput::Show {
                    icon: state.power.icon.clone().into(),
                    name: state
                        .power
                        .icon
                        .strip_suffix("-symbolic")
                        .unwrap()
                        .to_title_case()
                        .into(),
                    value: state.power.level,
                });
            }
        }
    }
}
