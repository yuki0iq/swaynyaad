use crate::bar::AppInput;
use crate::state::{AppState, Node, Screen};
use anyhow::{Context, Result};
use log::debug;
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock};
use swayipc_async::{Connection, Floating, NodeType};
use tokio::sync::mpsc;

pub async fn fetch(
    tx: &mpsc::UnboundedSender<AppInput>,
    conn: &mut Connection,
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
