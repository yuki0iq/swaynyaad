use eyre::{ensure, Context, Result};
use gtk::{gio, glib, prelude::*};
use log::{debug, info};
use relm4::gtk;
use tokio::process::Command;

pub async fn start() -> Result<()> {
    info!("Starting...");

    let action = gio::SimpleAction::new("subprocess", Some(glib::VariantTy::STRING_ARRAY));
    action.connect_activate(move |_action, value| {
        let Some(value) = value else { return };
        let Some(mut value) = value.get::<Vec<String>>() else {
            return;
        };
        if value.is_empty() {
            return;
        }
        let rest = value.split_off(1);
        let head = value.into_iter().next().unwrap();
        tokio::spawn(async move {
            debug!("Spawning {head:?} {rest:?}");
            let mut child = Command::new(head).args(rest).spawn().context("spawn")?;
            let exit_status = child.wait().await.context("wait")?;
            ensure!(exit_status.success());
            Ok(())
        });
    });
    relm4::main_application().add_action(&action);

    Ok(())
}
