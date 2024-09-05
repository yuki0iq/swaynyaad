use crate::bar::AppInput;
use crate::AppState;
use crate::{Pulse, PulseKind};
use alsa::mixer::{Mixer, Selem};
use alsa::poll::{pollfd, Descriptors};
use anyhow::{Context, Result};
use log::{debug, info, trace};
use std::sync::{Arc, RwLock};
use tokio::io::unix::AsyncFd;
use tokio::io::Interest;
use tokio::sync::mpsc;

async fn alsa_loop(pulse_tx: mpsc::UnboundedSender<(PulseKind, Pulse)>) -> Result<()> {
    info!("Starting ALSA main loop");
    let mixer = Mixer::new("default", false).context("alsa mixer create")?;

    info!("ALSA main loop ready");

    let mut fds: Vec<pollfd> = vec![];
    loop {
        mixer.handle_events().context("alsa mixer handle events")?;
        trace!("ALSA loop ticked");

        for elem in mixer.iter() {
            let Some(selem) = Selem::new(elem) else {
                continue;
            };

            let kind = match selem.get_id().get_name() {
                Ok("Master") => PulseKind::Sink,
                Ok("Capture") => PulseKind::Source,
                _ => continue,
            };

            pulse_tx
                .send((kind, Pulse::make(selem, kind)))
                .ok()
                .context("send alsa")?;
        }
        trace!("ALSA post-loop volume dispatch");

        let count = Descriptors::count(&mixer);
        fds.resize_with(count, || pollfd {
            fd: 0,
            events: 0,
            revents: 0,
        });
        Descriptors::fill(&mixer, &mut fds).context("fill descriptors")?;

        let mut futs = Vec::with_capacity(count);
        for pfd in &fds {
            let fd = pfd.fd;
            futs.push(tokio::spawn(async move {
                let interest = Interest::ERROR | Interest::READABLE;
                let afd = AsyncFd::with_interest(fd, interest).unwrap();
                let res = afd.ready(interest).await;
                res.map(|mut guard| guard.clear_ready())
            }));
        }
        let _ = futures::future::select_all(futs).await;
    }
}

pub async fn start(
    tx: mpsc::UnboundedSender<AppInput>,
    state: Arc<RwLock<AppState>>,
) -> Result<()> {
    info!("Starting ALSA mixer updater");
    let (pulse_tx, mut pulse_rx) = mpsc::unbounded_channel();
    relm4::spawn(alsa_loop(pulse_tx));

    info!("Started ALSA mixer, ready");

    while let Some((kind, pulse)) = pulse_rx.recv().await {
        let mut state = state.write().unwrap();
        let slot = match kind {
            PulseKind::Sink => &mut state.sink,
            PulseKind::Source => &mut state.source,
        };
        if *slot == pulse {
            continue;
        }
        debug!("ALSA state changed to {pulse:?}");
        *slot = pulse;
        tx.send(AppInput::Pulse(kind)).context("send pulse")?;
    }

    Ok(())
}
