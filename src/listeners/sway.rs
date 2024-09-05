use crate::bar::AppInput;
use crate::state::{AppState, Node, Screen, XkbLayout};
use anyhow::{bail, Context, Result};
use futures_lite::stream::StreamExt;
use log::{debug, info, trace};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, RwLock};
use swayipc_async::{Connection, Event, EventType};
use swayipc_async::{Floating, NodeType};
use tokio::sync::mpsc;

pub async fn start(
    tx: mpsc::UnboundedSender<AppInput>,
    state: Arc<RwLock<AppState>>,
) -> Result<()> {
    info!("Starting sway listener");

    let mut conn = Connection::new().await.context("initial connection")?;
    let mut stream = Connection::new()
        .await
        .context("event connection")?
        .subscribe([
            EventType::Input,
            EventType::Output,
            EventType::Workspace,
            EventType::Window,
        ])
        .await
        .context("subscribe to events")?;

    info!("Sway listener ready");

    sway_fetch_output(&tx, &mut conn, Arc::clone(&state))
        .await
        .context("init output")?;
    sway_fetch_input(&tx, &mut conn, Arc::clone(&state))
        .await
        .context("init input")?;

    while let Some(event) = stream.next().await {
        let Ok(event) = event else { continue };
        trace!("Received sway event {event:?}");
        match event {
            Event::Input(_) => sway_fetch_input(&tx, &mut conn, Arc::clone(&state))
                .await
                .context("fetch input")?,
            Event::Output(_) => sway_fetch_output(&tx, &mut conn, Arc::clone(&state))
                .await
                .context("fetch output")?,
            Event::Window(_) | Event::Workspace(_) => {
                sway_fetch_workspace(&tx, &mut conn, Arc::clone(&state))
                    .await
                    .context("fetch workspace")?
            }
            _ => bail!("Unexpected event"),
        }
    }

    Ok(())
}

async fn sway_fetch_input(
    tx: &mpsc::UnboundedSender<AppInput>,
    conn: &mut swayipc_async::Connection,
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

async fn sway_fetch_output(
    tx: &mpsc::UnboundedSender<AppInput>,
    conn: &mut swayipc_async::Connection,
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

    sway_fetch_workspace(tx, conn, state).await?;

    Ok(())
}

async fn sway_fetch_workspace(
    tx: &mpsc::UnboundedSender<AppInput>,
    conn: &mut swayipc_async::Connection,
    state: Arc<RwLock<AppState>>,
) -> Result<()> {
    debug!("Fetching workspace information");

    let workspaces = conn.get_workspaces().await.context("get workspaces")?;
    let workspaces_existing = workspaces.iter().map(|ws| ws.num).collect::<BTreeSet<_>>();
    let workspaces_urgent = workspaces
        .iter()
        .filter(|ws| ws.urgent)
        .map(|ws| ws.num)
        .collect::<Vec<_>>();

    let outputs = conn.get_outputs().await.context("get outputs")?;
    let screen_focused = outputs
        .iter()
        .find(|output| output.focused)
        .map(|output| output.name.clone());

    let tree = conn.get_tree().await.context("get tree")?;

    let mut screens = HashMap::new();
    for output in outputs {
        // This is O(total_nodes), and not O(workspaces)
        let workspace = tree.find_as_ref(|node| {
            node.node_type == NodeType::Workspace && node.name == output.current_workspace
        });
        let focused = workspace.and_then(|ws| {
            ws.find_focused_as_ref(|node| {
                matches!(node.node_type, NodeType::FloatingCon | NodeType::Con)
                    && node.nodes.is_empty()
            })
        });
        screens.insert(
            output.name,
            Screen {
                workspace: output.current_workspace,
                focused: focused.map(|node| Node {
                    shell: serde_json::to_string(&node.shell).unwrap(),
                    floating: matches!(
                        node.floating,
                        Some(Floating::AutoOn) | Some(Floating::UserOn)
                    ),
                    app_id: node.app_id.clone().or_else(|| {
                        Some(format!(
                            "{} [X11]",
                            node.window_properties.as_ref()?.class.as_ref()?
                        ))
                    }),
                }),
            },
        );
    }

    {
        let mut state = state.write().unwrap();
        state.workspaces_urgent = workspaces_urgent;
        state.workspaces_existing = workspaces_existing;
        state.screen_focused = screen_focused;
        state.screens = screens;
    }
    tx.send(AppInput::Workspaces).context("send workspaces")?;

    Ok(())
}
