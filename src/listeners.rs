use crate::bar::AppInput;
use crate::AppState;
use log::trace;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

mod bus;
mod sound;
mod sway;
mod time;

pub fn start(tx: mpsc::UnboundedSender<AppInput>, state: Arc<RwLock<AppState>>) {
    trace!("Spawning listeners...");
    relm4::spawn(sway::start(tx.clone(), Arc::clone(&state)));
    relm4::spawn(time::start(tx.clone(), Arc::clone(&state)));
    relm4::spawn(sound::start(tx.clone(), Arc::clone(&state)));
    relm4::spawn(bus::start(tx.clone(), Arc::clone(&state)));
}
