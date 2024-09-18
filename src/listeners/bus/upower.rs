use crate::bar::AppInput;
use crate::state::{AppState, Power};
use eyre::{Context, Result};
use futures_lite::stream::StreamExt;
use log::{debug, info};
use std::sync::{Arc, RwLock};
use tokio::sync::{mpsc, Notify};
use upower_dbus::{BatteryState, BatteryType, DeviceProxy, UPowerProxy};

async fn upower_state(
    tx: mpsc::UnboundedSender<AppInput>,
    state: Arc<RwLock<AppState>>,
    device: DeviceProxy<'_>,
) -> Result<()> {
    let present = device.is_present().await?;
    let level = device.percentage().await?;

    let bat_state = device.state().await?;
    let charging = matches!(
        bat_state,
        BatteryState::PendingCharge | BatteryState::Charging | BatteryState::FullyCharged
    );

    let bat_type = device.type_().await?;
    let icon = match bat_type {
        BatteryType::LinePower => "ac-adapter-symbolic".into(),
        _ => match bat_state {
            BatteryState::Empty => "battery-empty-symbolic".into(),
            BatteryState::FullyCharged => "battery-full-charged-symbolic".into(),
            BatteryState::PendingCharge
            | BatteryState::Charging
            | BatteryState::PendingDischarge
            | BatteryState::Discharging => format!(
                "battery-level-{}{}-symbolic",
                (level / 10.).floor() * 10.,
                if charging { "-charging" } else { "" }
            ),
            _ => "battery-missing-symbolic".into(),
        },
    };

    let changed;
    {
        let power = &mut state.write().unwrap().power;
        let new_power = Power {
            present,
            level,
            icon,
            charging,
        };

        changed = power.present != new_power.present || power.charging != new_power.charging;

        debug!("UPower state: {new_power:?}, changed? {changed}");

        *power = new_power;
    }

    tx.send(AppInput::Power).context("upower init")?;
    if changed {
        tx.send(AppInput::PowerChanged).context("upower changed")?;
    }

    Ok(())
}

async fn stream_notify<S, T>(notify: Arc<Notify>, mut stream: S)
where
    S: futures::stream::Stream<Item = T> + Unpin,
{
    while stream.next().await.is_some() {
        notify.notify_one();
    }
}

pub async fn start(
    tx: mpsc::UnboundedSender<AppInput>,
    state: Arc<RwLock<AppState>>,
    conn: zbus::Connection,
) -> Result<()> {
    debug!("Starting UPower listeners...");

    let upower = UPowerProxy::new(&conn).await.context("bind to upower")?;
    let device = upower.get_display_device().await?;

    debug!("Connected to UPower instance");

    let notify = Arc::new(Notify::new());

    tokio::spawn(stream_notify(
        Arc::clone(&notify),
        device.receive_is_present_changed().await,
    ));
    tokio::spawn(stream_notify(
        Arc::clone(&notify),
        device.receive_percentage_changed().await,
    ));
    tokio::spawn(stream_notify(
        Arc::clone(&notify),
        device.receive_icon_name_changed().await,
    ));

    info!("Started UPower listeners, ready");

    loop {
        debug!("UPower state changed");
        upower_state(tx.clone(), Arc::clone(&state), device.clone()).await?;

        let _ = notify.notified().await;
    }
}
