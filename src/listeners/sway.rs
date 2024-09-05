use crate::bar::AppInput;
use crate::state::AppState;
use anyhow::{bail, Context, Result};
use futures_lite::stream::StreamExt;
use log::{info, trace};
use std::sync::{Arc, RwLock};
use swayipc_async::{Connection, Event, EventType};
use tokio::sync::mpsc;

mod input;
mod output;
mod workspace;

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

    output::fetch(&tx, &mut conn, Arc::clone(&state)).await?;
    input::fetch(&tx, &mut conn, Arc::clone(&state)).await?;

    while let Some(event) = stream.next().await {
        let Ok(event) = event else { continue };
        trace!("Received sway event {event:?}");
        let state = Arc::clone(&state);
        match event {
            Event::Input(_) => input::fetch(&tx, &mut conn, state).await,
            Event::Output(_) => output::fetch(&tx, &mut conn, state).await,
            Event::Window(_) | Event::Workspace(_) => workspace::fetch(&tx, &mut conn, state).await,
            _ => bail!("Unexpected event"),
        }?
    }

    Ok(())
}
