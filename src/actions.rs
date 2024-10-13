use log::info;
use relm4::actions::*;

mod sway;

relm4::new_action_group!(pub Group, "app");
pub type RelmGroup = RelmActionGroup<Group>;

pub fn setup() {
    info!("Setting up...");

    let mut group = RelmGroup::new();
    sway::setup(&mut group);
    group.register_for_main_application();
}
