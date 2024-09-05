use crate::bar::AppInput;
use crate::state::AppState;
use anyhow::{Context, Result};
use log::{debug, info};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

mod upower;

pub async fn start(
    tx: mpsc::UnboundedSender<AppInput>,
    state: Arc<RwLock<AppState>>,
) -> Result<()> {
    debug!("Starting zbus listeners...");

    let conn = zbus::Connection::system()
        .await
        .context("connect to system bus")?;

    tokio::spawn(upower::start(tx.clone(), Arc::clone(&state), conn.clone()));

    info!("Started zbus listeners");
    Ok(())
}
