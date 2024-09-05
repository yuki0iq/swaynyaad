use gtk::{gdk, prelude::*};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use log::info;
use relm4::prelude::*;

pub struct CriticalModel {
    pub monitor: gdk::Monitor,
}

#[derive(Debug, Clone)]
pub enum CriticalInput {
    // TODO: support more than one critical notifications
    Show(String),
    Hide,
}

#[relm4::component(pub)]
impl Component for CriticalModel {
    type Init = CriticalModel;
    type Input = CriticalInput;
    type Output = ();
    type CommandOutput = ();

    view! {
        #[name(window)] gtk::Window {
            init_layer_shell: (),
            set_monitor: &model.monitor,
            set_layer: Layer::Overlay,
            set_anchor: (Edge::Top, true),
            set_margin: (Edge::Top, 40),
            add_css_class: "critical",
            set_visible: false,

            #[name(text)] gtk::Label,
        }
    }

    fn init(
        model: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        info!("Creating Critical for {:?}", model.monitor.connector());
        let widgets = view_output!();

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
            CriticalInput::Hide => ui.window.set_visible(false),
            CriticalInput::Show(state) => {
                ui.window.set_visible(true);
                ui.text.set_text(&state);
            }
        }
    }
}
