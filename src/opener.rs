use anyhow::{ensure, Context, Result};
use smol::process::Command;
use smol::stream::StreamExt;
use swayipc_async::{Connection, EventType};

async fn update_monitor_state(conn: &mut Connection) -> Result<()> {
    let outputs = conn.get_outputs().await.context("Get outputs")?;

    let _status = Command::new("eww")
        .arg("--restart")
        .arg("close-all")
        .spawn()
        .context("spawn close-all")?
        .status()
        .await
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
            .status()
            .await
            .with_context(|| format!("open on monitor {} wasn't running", output.name))?
            .success();
        ensure!(status, "open on monitor {} failed", output.name);
    }

    Ok(())
}

pub async fn open_bars(mut conn: Connection) -> Result<()> {
    let mut stream = Connection::new()
        .await
        .context("Create another connection")?
        .subscribe([EventType::Output])
        .await
        .context("Subscribe to events")?;

    // Do-while loop.
    while {
        update_monitor_state(&mut conn)
            .await
            .context("Update in response to input")?;
        true
    } && let Some(event) = stream.next().await
    {
        let _ = event.context("invalid event")?;
    }
    Ok(())
}
