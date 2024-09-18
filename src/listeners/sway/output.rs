use crate::bar::AppInput;
use crate::state::AppState;
use eyre::{Context, Result};
use log::debug;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use swayipc_async::Connection;
use tokio::sync::mpsc;

pub async fn fetch(
    tx: &mpsc::UnboundedSender<AppInput>,
    conn: &mut Connection,
    state: Arc<RwLock<AppState>>,
) -> Result<()> {
    debug!("Fetching outputs information");

    let outputs = conn
        .get_outputs()
        .await
        .context("get outputs")?
        .into_iter()
        .map(|out| out.name)
        .collect::<HashSet<_>>();

    tx.send(AppInput::Outputs(outputs))
        .context("send outputs")?;

    super::workspace::fetch(tx, conn, state).await?;

    Ok(())
}
