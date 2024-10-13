use crate::bar::AppInput;
use crate::state::{AppState, XkbLayout};
use eyre::{Context, OptionExt, Result};
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

    let layouts = inputs
        .iter()
        .find(|input| input.input_type == "keyboard")
        .map(|input| input.xkb_layout_names.clone())
        .ok_or_eyre("keyboard not found")?;

    let layout_index = inputs
        .iter()
        .find_map(|input| input.xkb_active_layout_index)
        .unwrap_or(0);
    let layout_name: &str = layouts
        .get(layout_index as usize)
        .map(|name| &name[..])
        .unwrap_or("XX Not found");

    {
        let mut state = state.write().unwrap();

        // TODO: more correct short name
        state.layout = XkbLayout {
            name: layout_name[..2].to_ascii_lowercase(),
            description: layout_name.into(),
        };
        tx.send(AppInput::Layout).context("send layout")?;

        if state.layouts != layouts {
            state.layouts = layouts;
            tx.send(AppInput::LayoutList).context("send layout list")?;
        }
    }

    Ok(())
}
