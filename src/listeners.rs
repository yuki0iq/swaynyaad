use crate::bar::AppInput;
use crate::state::AppState;
use log::trace;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

mod sound;
mod sway;
mod time;
mod upower;

pub fn start(tx: mpsc::UnboundedSender<AppInput>, state: Arc<RwLock<AppState>>) {
    trace!("Spawning listeners...");
    relm4::spawn_local(sway::start(tx.clone(), Arc::clone(&state)));
    tokio::spawn(time::start(tx.clone(), Arc::clone(&state)));
    tokio::spawn(sound::start(tx.clone(), Arc::clone(&state)));
    relm4::spawn_local(upower::start(tx.clone(), Arc::clone(&state)));
}
