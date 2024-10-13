use super::{Group, RelmGroup};
use eyre::{Context, Result};
use log::info;
use relm4::actions::*;
use swayipc_async::Connection;

relm4::new_stateful_action!(ChangeLayoutAction, Group, "xkb_switch_layout", u64, u64);

async fn run_single_command<T: AsRef<str>>(payload: T) -> Result<()> {
    let mut conn = Connection::new().await.context("connect to sway")?;
    conn.run_command(payload).await.context("execute command")?;
    Ok(())
}

fn run_single_command_sync<T: AsRef<str> + Send + 'static>(payload: T) {
    relm4::spawn(async { run_single_command(payload).await.unwrap() });
}

pub fn setup(group: &mut RelmGroup) {
    info!("Setting up...");

    group.add_action(
        RelmAction::<ChangeLayoutAction>::new_stateful_with_target_value(
            &0,
            |_action, state, value| {
                *state = value;
                run_single_command_sync(format!("input type:keyboard xkb_switch_layout {value}"));
            },
        ),
    );
}
