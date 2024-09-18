use crate::bar::AppInput;
use crate::state::AppState;
use eyre::{bail, Context, Result};
use chrono::offset::Local;
use log::{info, trace};
use rustix::system;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

pub async fn start(
    tx: mpsc::UnboundedSender<AppInput>,
    state: Arc<RwLock<AppState>>,
) -> Result<()> {
    let mut timer = tokio::time::interval(Duration::from_secs(1));
    info!("Started timer-based listener");

    loop {
        trace!("Timer ticked");

        state.write().unwrap().time = Local::now();
        tx.send(AppInput::Time).context("send time")?;

        {
            let sysinfo = system::sysinfo();

            let meminfo = File::open("/proc/meminfo").await.context("read meminfo")?;
            let mut meminfo = BufReader::new(meminfo).lines();
            let mut total_ram: usize = 1;
            let mut available_ram: usize = 0;
            let mut count_fields = 2;
            while let Some(line) = meminfo.next_line().await.context("line meminfo")? {
                let entries = line.split_whitespace().collect::<Vec<_>>();
                match entries[..] {
                    [name, value, _unit] => match name {
                        "MemTotal:" => {
                            total_ram = value.parse().context("bad total_ram")?;
                            count_fields -= 1;
                        }
                        "MemAvailable:" => {
                            available_ram = value.parse().context("bad available_ram")?;
                            count_fields -= 1;
                        }
                        _ => {}
                    },
                    [_name, _value] => {}
                    _ => bail!("/proc/meminfo has unexpected format"),
                }

                if count_fields == 0 {
                    break;
                }
            }

            let load_average = sysinfo.loads[0] as f64 / 65536.;
            let memory_usage = 1. - available_ram as f64 / total_ram as f64;

            let mut state = state.write().unwrap();
            state.load_average = load_average;
            state.memory_usage = memory_usage;
            tx.send(AppInput::Sysinfo).context("send sysinfo")?;
        }

        let _ = timer.tick().await;
    }
}
