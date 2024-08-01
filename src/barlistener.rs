use anyhow::{Context, Result};
use serde::Serialize;
use smol::stream::StreamExt;
use std::collections::{BTreeSet, HashMap};
use swayipc_async::{Connection, EventType, NodeType, ShellType};

#[derive(Debug, Serialize)]
struct Screen {
    workspace: Option<String>,
    name: Option<String>,
    shell: Option<ShellType>,
    app_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct State {
    layout_name: Option<String>,
    layout_short_name: String,
    workspaces_existing: BTreeSet<i32>,
    workspaces_urgent: Vec<i32>,
    screen_focused: Option<String>,
    screens: HashMap<String, Screen>,
}

async fn update_bar_state(conn: &mut Connection) -> Result<()> {
    let layout_name = conn
        .get_inputs()
        .await
        .context("Get inputs")?
        .into_iter()
        .find_map(|input| input.xkb_active_layout_name);

    let workspaces = conn.get_workspaces().await.context("Get workspaces")?;
    let workspaces_existing = workspaces.iter().map(|ws| ws.num).collect::<BTreeSet<_>>();
    let workspaces_urgent = workspaces
        .iter()
        .filter(|ws| ws.urgent)
        .map(|ws| ws.num)
        .collect::<Vec<_>>();

    let outputs = conn.get_outputs().await.context("Get outputs")?;
    let screen_focused = outputs
        .iter()
        .find(|output| output.focused)
        .map(|output| output.name.clone());

    let tree = conn.get_tree().await.context("Get tree")?;

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
                name: focused.and_then(|node| node.name.clone()),
                shell: focused.and_then(|node| node.shell),
                app_id: focused.and_then(|node| node.app_id.clone()).or_else(|| {
                    focused
                        .and_then(|node| node.window_properties.as_ref())
                        .and_then(|prop| prop.class.as_ref())
                        .map(|class| format!("{class} [X11]"))
                }),
            },
        );
    }

    let state = State {
        layout_short_name: layout_name
            .as_ref()
            .map(|layout| layout[..2].to_ascii_lowercase())
            .unwrap_or_else(|| "xx".into()),
        layout_name,
        workspaces_existing,
        workspaces_urgent,
        screen_focused,
        screens,
    };

    println!(
        "{}",
        serde_json::to_string(&state).context("Failed to serialize")?
    );

    Ok(())
}

pub async fn listen_for_bar(mut conn: Connection) -> Result<()> {
    let mut stream = Connection::new()
        .await
        .context("Create another connection")?
        .subscribe([EventType::Workspace, EventType::Window, EventType::Input])
        .await
        .context("Subscribe to events")?;

    // Do-while loop.
    while {
        update_bar_state(&mut conn)
            .await
            .context("Update in response to input")?;
        true
    } && let Some(event) = stream.next().await
    {
        let _ = event.context("invalid event")?;
    }

    Ok(())
}
