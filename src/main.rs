use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};
use std::process::Command;
use std::{env, process};
use swayipc::{Connection, Event, EventType, NodeType, ShellType};

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

fn update_bar_state(conn: &mut Connection) -> Result<()> {
    let layout_name = conn
        .get_inputs()
        .context("Get inputs")?
        .into_iter()
        .find_map(|input| input.xkb_active_layout_name);

    let workspaces = conn.get_workspaces().context("Get workspaces")?;
    let workspaces_existing = workspaces.iter().map(|ws| ws.num).collect::<BTreeSet<_>>();
    let workspaces_urgent = workspaces
        .iter()
        .filter(|ws| ws.urgent)
        .map(|ws| ws.num)
        .collect::<Vec<_>>();

    let outputs = conn.get_outputs().context("Get outputs")?;
    let screen_focused = outputs
        .iter()
        .find(|output| output.focused)
        .map(|output| output.name.clone());

    let tree = conn.get_tree().context("Get tree")?;

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
                app_id: focused.and_then(|node| node.app_id.clone()),
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

fn listen_for_bar(mut conn: Connection) -> Result<()> {
    update_bar_state(&mut conn).context("Report initial state")?;

    let events = Connection::new()
        .context("Create another connection")?
        .subscribe([EventType::Workspace, EventType::Window, EventType::Input])
        .context("Subscribe to events")?;
    for event in events {
        let event = event.context("Invalid event")?;

        match event {
            Event::Input(_) | Event::Workspace(_) | Event::Window(_) => {
                update_bar_state(&mut conn).context("Update in response to input")?;
            }
            _ => {
                bail!("Got unexpected event from sway: {event:?}");
            }
        }
    }

    Ok(())
}

fn update_monitor_state(conn: &mut Connection) -> Result<()> {
    let outputs = conn.get_outputs().context("Get outputs")?;

    let _status = Command::new("eww")
            .arg("--restart")
            .arg("close-all")
            .spawn()
            .context("spawn close-all")?
            .wait()
            .context("close-all wasn't running")?
            .success();
    // ensure!(status, "close-all failed");
    // close-all _may_ fail

    for (idx, output) in outputs.iter().enumerate() {
        let status = Command::new("eww")
            .arg("open")
            .arg("bar")
            .arg("--screen")
            .arg(idx.to_string())
            .arg("--id")
            .arg(format!("bar-{}", output.name))
            .arg("--arg")
            .arg(format!("monitor={}", output.name))
            .spawn()
            .with_context(|| format!("spawn open on monitor {}", output.name))?
            .wait()
            .with_context(|| format!("open on monitor {} wasn't running", output.name))?
            .success();
        ensure!(status, "open on monitor {} failed", output.name);
    }

    Ok(())
}

fn open_bars(mut conn: Connection) -> Result<()> {
    update_monitor_state(&mut conn).context("Initial bar launch")?;

    let events = Connection::new()
        .context("Create another connection")?
        .subscribe([EventType::Output])
        .context("Subscribe to events")?;
    for event in events {
        let event = event.context("Invalid event")?;

        match event {
            Event::Output(_) => {
                update_monitor_state(&mut conn).context("Update in response to input")?;
            }
            _ => {
                bail!("Got unexpected event from sway: {event:?}");
            }
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    let conn = Connection::new().context("Create connection")?;

    let args = env::args().collect::<Vec<_>>();
    match args.get(1).map(|arg| arg.as_ref()) {
        Some("--listener") => listen_for_bar(conn)?,
        Some("--opener") => open_bars(conn)?,
        Some(_) => {
            eprintln!("Unknown command-line option supplied");
            process::exit(1);
        }
        None => {
            eprintln!("You are not supposed to run this binary directly");
            process::exit(1);
        }
    }

    Ok(())
}
