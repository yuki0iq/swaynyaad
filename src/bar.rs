use crate::changer::{ChangerInput, ChangerModel};
use crate::critical::{CriticalInput, CriticalModel};
use crate::state::{AppState, PulseKind};
use gtk::{gdk, gio, prelude::*, Align};
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
    LayoutList,
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
                        #[wrap(Some)] set_child = &gtk::Box {
                            // NOTE: The spacing is higher than between icons!
                            set_spacing: 16,
                            #[name(date)] gtk::Label,
                            #[name(time)] gtk::Label,
                        },
                        #[wrap(Some)] set_popover = &gtk::Popover {
                            // TODO styles and date.
                            #[wrap(Some)] set_child = &gtk::Calendar,
                        },
                    },
                    gtk::MenuButton {
                        #[wrap(Some)] #[name(layout)] set_child = &gtk::Label,
                        #[wrap(Some)] #[name(layout_menu)] set_popover = &gtk::PopoverMenu::from_model(None::<&gio::Menu>),
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
        root.set_application(Some(&relm4::main_application()));
        let widgets = view_output!();

        for event in [
            AppInput::Layout,
            AppInput::LayoutList,
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
            AppInput::Layout => {
                ui.layout.set_label(&state.layout.name);
            }
            AppInput::LayoutList => {
                // XXX Rebuilding a menu seems like a bad taste

                let menu = gio::Menu::new();

                let layout_menu = gio::Menu::new();
                for (index, layout_name) in state.layouts.iter().enumerate() {
                    let item = gio::MenuItem::new(None, None);
                    item.set_label(Some(layout_name));
                    item.set_action_and_target_value(
                        Some("app.xkb_switch_layout"),
                        Some(&(index as u64).into()),
                    );
                    layout_menu.append_item(&item);
                }
                menu.append_section(None, &layout_menu);

                ui.layout_menu.set_menu_model(Some(&menu));
            }
            AppInput::Time => {
                if std::env::var_os("alternative_time").is_some() {
                    // difference between Apr 12, 1961 06:07 UTC and Jan 1, 0000 00:00 UTC
                    // see https://play.rust-lang.org/?version=stable&mode=debug&edition=2021&gist=c94dab72cb3a36449be9284e6ea08bd4
                    const TERRA_EPOCH: chrono::TimeDelta = chrono::TimeDelta::seconds(61891970820);
                    let terra = state.time.to_utc() - TERRA_EPOCH;

                    ui.date
                        .set_label(&terra.format("Terra %Y day %j").to_string());
                    ui.time.set_label(&terra.format("%T").to_string());
                } else {
                    ui.date
                        .set_label(&state.time.format("%a %b %-d").to_string());
                    ui.time.set_label(&state.time.format("%T").to_string());
                }
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
