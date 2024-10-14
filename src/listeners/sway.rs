use crate::bar::AppInput;
use crate::state::AppState;
use eyre::{bail, Context, Result};
use futures_lite::stream::StreamExt;
use gtk4::prelude::ActionMapExt;
use log::{error, info, trace};
use relm4::gtk::{gio, glib};
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

    let (command_tx, mut command_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut conn = Connection::new()
            .await
            .context("initial connection")
            .unwrap();
        while let Some(payload) = command_rx.recv().await {
            trace!("Requesting {payload}...");
            let res = conn.run_command(&payload).await;
            if res.is_err() {
                error!("got {res:?} in response to {payload}");
            }
        }
    });

    let action_switch_layout = gio::SimpleAction::new_stateful(
        "xkb_switch_layout",
        Some(glib::VariantTy::INT32),
        &0.into(),
    );
    let command_tx_ = command_tx.clone();
    action_switch_layout.connect_change_state(move |_action, value| {
        log::trace!("Switch layout action triggered with new value {value:?}");
        let Some(value) = value else { return };
        let Some(value) = value.get::<i32>() else {
            return;
        };
        command_tx_
            .send(format!("input type:keyboard xkb_switch_layout {value}"))
            .expect("send command");
    });
    relm4::main_application().add_action(&action_switch_layout);

    let (new_tx, mut rx) = mpsc::unbounded_channel();
    relm4::spawn_local(async move {
        while let Some(event) = rx.recv().await {
            trace!("Forwarding event {event:?}");
            if let AppInput::Layout(idx) = event {
                action_switch_layout.set_state(&idx.into());
            }
            tx.send(event).expect("forward event");
        }
    });
    let tx = new_tx;

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
