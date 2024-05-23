use anyhow::{bail, Context, Result};
use serde::Serialize;
use swayipc::{Connection, Event, EventType, ShellType};

#[derive(Debug, Serialize)]
struct State {
    layout_name: Option<String>,
    layout_short_name: String,
    workspaces_existing: Vec<i32>,
    workspaces_urgent: Vec<i32>,
    // TODO respect multi-monitor setup
    workspace_focused: i32,
    window_focused_name: Option<String>,
    window_focused_shell: Option<ShellType>,
    window_focused_app_id: Option<String>,
}

fn update_state(conn: &mut Connection) -> Result<()> {
    let layout_name = conn
        .get_inputs()
        .context("Get inputs")?
        .into_iter()
        .find_map(|input| input.xkb_active_layout_name);

    let workspaces = conn.get_workspaces().context("Get workspaces")?;
    let workspaces_existing = workspaces.iter().map(|ws| ws.num).collect::<Vec<_>>();
    let workspaces_urgent = workspaces
        .iter()
        .filter(|ws| ws.urgent)
        .map(|ws| ws.num)
        .collect::<Vec<_>>();
    let workspace_focused = workspaces
        .iter()
        .find(|ws| ws.focused)
        .map(|ws| ws.num)
        .context("No focused workspace")?;

    let tree = conn.get_tree().context("Get tree")?;
    let focused = tree.find_as_ref(|node| node.focused);

    let state = State {
        layout_short_name: layout_name
            .as_ref()
            .map(|layout| layout[..2].to_ascii_lowercase().into())
            .unwrap_or_else(|| "xx".into()),
        layout_name,
        workspaces_existing,
        workspaces_urgent,
        workspace_focused,
        window_focused_name: focused.and_then(|node| node.name.clone()),
        window_focused_shell: focused.and_then(|node| node.shell),
        window_focused_app_id: focused.and_then(|node| node.app_id.clone()),
    };

    println!(
        "{}",
        serde_json::to_string(&state).context("Failed to serialize")?
    );

    Ok(())
}

fn main() -> Result<()> {
    let mut conn = Connection::new().context("Create connection")?;
    update_state(&mut conn).context("Report initial state")?;

    let events = Connection::new()
        .context("Create another connection")?
        .subscribe([EventType::Workspace, EventType::Window, EventType::Input])
        .context("Subscribe to events")?;
    for event in events {
        let event = event.context("Invalid event")?;

        match event {
            Event::Input(_) | Event::Workspace(_) | Event::Window(_) => {
                update_state(&mut conn).context("Update in response to input")?;
            }
            _ => {
                bail!("Got unexpected event from sway: {event:?}");
            }
        }
    }

    Ok(())
}
