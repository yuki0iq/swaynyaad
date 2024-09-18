use crate::bar::AppInput;
use crate::state::{AppState, XkbLayout};
use eyre::{Context, Result};
use log::debug;
use std::sync::{Arc, RwLock};
use swayipc_async::Connection;
use tokio::sync::mpsc;

pub async fn fetch(
    tx: &mpsc::UnboundedSender<AppInput>,
    conn: &mut Connection,
    state: Arc<RwLock<AppState>>,
) -> Result<()> {
    debug!("Fetching input information");

    let inputs = conn.get_inputs().await.context("get inputs")?;

    let layout_name = inputs
        .iter()
        .find_map(|input| input.xkb_active_layout_name.as_ref());

    state.write().unwrap().layout = XkbLayout {
        name: layout_name
            .map(|layout| layout[..2].to_ascii_lowercase())
            .unwrap_or_else(|| "xx".into()),
        description: layout_name
            .cloned()
            .unwrap_or_else(|| "Unknown layout".into()),
    };
    tx.send(AppInput::Layout).context("send layout")?;
    Ok(())
}
