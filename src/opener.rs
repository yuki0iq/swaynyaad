use anyhow::{bail, ensure, Context, Result};
use std::process::Command;
use swayipc::{Connection, Event, EventType};

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

pub fn open_bars(mut conn: Connection) -> Result<()> {
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
