use crate::bar::AppInput;
use crate::state::{AppState, Power};
use eyre::{Context, OptionExt, Result};
use log::{debug, info};
use relm4::gtk::glib;
use std::sync::{Arc, RwLock};
use tokio::sync::{mpsc, Notify};
use upower_glib::{Client, ClientExt, Device, DeviceExt, DeviceKind, DeviceState};

fn upower_state(
    tx: &mpsc::UnboundedSender<AppInput>,
    state: &mut AppState,
    device: &Device,
) -> Result<()> {
    /// XXX: This should be moved to upower_glib crate.
    use glib::translate::FromGlib;

    let present = device.is_present();
    let level = device.percentage();

    let bat_state = unsafe { DeviceState::from_glib(device.state() as _) };
    let charging = matches!(
        bat_state,
        DeviceState::PendingCharge | DeviceState::Charging | DeviceState::FullyCharged
    );

    let bat_type = unsafe { DeviceKind::from_glib(device.kind() as _) };
    let icon = match bat_type {
        DeviceKind::LinePower => "ac-adapter-symbolic".into(),
        _ => match bat_state {
            DeviceState::Empty => "battery-empty-symbolic".into(),
            DeviceState::FullyCharged => "battery-full-charged-symbolic".into(),
            DeviceState::PendingCharge
            | DeviceState::Charging
            | DeviceState::PendingDischarge
            | DeviceState::Discharging => format!(
                "battery-level-{}{}-symbolic",
                (level / 10.).floor() * 10.,
                if charging { "-charging" } else { "" }
            ),
            _ => "battery-missing-symbolic".into(),
        },
    };

    let changed;
    {
        let power = &mut state.power;
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

pub async fn start(
    tx: mpsc::UnboundedSender<AppInput>,
    state: Arc<RwLock<AppState>>,
) -> Result<()> {
    debug!("Starting UPower listeners...");

    let client = Client::new_future().await.context("bind to upower")?;
    let device = client.display_device().ok_or_eyre("no display device")?;

    debug!("Connected to UPower instance");

    let notify = Arc::new(Notify::new());

    device.connect_is_present_notify(glib::clone!(
        #[strong]
        notify,
        move |_| notify.notify_one()
    ));
    device.connect_percentage_notify(glib::clone!(
        #[strong]
        notify,
        move |_| notify.notify_one()
    ));
    device.connect_icon_name_notify(glib::clone!(
        #[strong]
        notify,
        move |_| notify.notify_one()
    ));

    info!("Started UPower listeners, ready");

    loop {
        upower_state(&tx, &mut state.write().unwrap(), &device).context("initial report")?;

        let _ = notify.notified().await;
    }
}
