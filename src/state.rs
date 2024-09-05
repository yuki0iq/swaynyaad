use alsa::mixer::{Selem, SelemChannelId};
use chrono::{offset::Local, DateTime};
use std::collections::{BTreeSet, HashMap};

#[derive(Debug, Default, Clone, PartialEq)]
pub struct XkbLayout {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Default)]
pub struct Node {
    pub shell: String,
    pub app_id: Option<String>,
    pub floating: bool,
}

#[derive(Debug, Default)]
pub struct Screen {
    pub workspace: Option<String>,
    pub focused: Option<Node>,
}

#[derive(Debug, Clone, Copy)]
pub enum PulseKind {
    Sink,
    Source,
}

#[derive(Debug, Default, PartialEq)]
pub struct Pulse {
    pub muted: bool,
    pub volume: i64,
    pub icon: String,
}

impl Pulse {
    fn parse(selem: Selem, kind: PulseKind) -> (i64, bool) {
        let has_volume = match kind {
            PulseKind::Sink => selem.has_playback_volume(),
            PulseKind::Source => selem.has_capture_volume(),
        };
        if !has_volume {
            // This device is probably nonexistent. Returning anything "normal" is okay
            return (0, true);
        }

        let (volume_low, volume_high) = match kind {
            PulseKind::Sink => selem.get_playback_volume_range(),
            PulseKind::Source => selem.get_capture_volume_range(),
        };

        let mut globally_muted = match kind {
            PulseKind::Sink => selem.has_playback_switch(),
            PulseKind::Source => selem.has_capture_switch(),
        };

        let mut channel_count = 0;
        let mut acc_volume = 0;
        for scid in SelemChannelId::all() {
            let Ok(cur_volume) = (match kind {
                PulseKind::Sink => selem.get_playback_volume(*scid),
                PulseKind::Source => selem.get_capture_volume(*scid),
            }) else {
                continue;
            };

            let cur_muted = match kind {
                PulseKind::Sink => selem.get_playback_switch(*scid),
                PulseKind::Source => selem.get_capture_switch(*scid),
            } == Ok(0);

            globally_muted = globally_muted && cur_muted;
            channel_count += 1;
            if !cur_muted {
                acc_volume += cur_volume - volume_low;
            }
        }

        let volume = 100 * acc_volume / (volume_high - volume_low) / channel_count;
        (volume, globally_muted)
    }

    pub fn make(selem: Selem, kind: PulseKind) -> Self {
        let (volume, muted) = Self::parse(selem, kind);

        let icon = format!(
            "{}-volume-{}",
            match kind {
                PulseKind::Sink => "audio",
                PulseKind::Source => "mic",
            },
            match volume {
                0 => "muted",
                _ if muted => "muted",
                v if v <= 25 => "low",
                v if v <= 50 => "medium",
                v if v <= 100 => "high",
                _ => "high",
            }
        );

        Self {
            icon,
            muted,
            volume,
        }
    }
}

#[derive(Debug, Default)]
pub struct Power {
    pub present: bool,
    pub charging: bool,
    pub level: f64,
    pub icon: String,
}

impl Power {
    pub fn is_critical(&self) -> bool {
        self.present && !self.charging && self.level < 10.
    }
}

#[derive(Debug, Default)]
pub struct AppState {
    pub layout: XkbLayout,
    pub time: DateTime<Local>,
    pub workspaces_urgent: Vec<i32>,
    pub workspaces_existing: BTreeSet<i32>,
    pub screen_focused: Option<String>,
    pub screens: HashMap<String, Screen>,
    pub load_average: f64,
    pub memory_usage: f64,
    pub sink: Pulse,
    pub source: Pulse,
    pub power: Power,
}
