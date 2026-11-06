use serde::{Deserialize, Serialize};

use crate::output::Output;

/// Playback state of a zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayState {
    Playing,
    Paused,
    Loading,
    Stopped,
}

/// Loop mode setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopMode {
    Loop,
    LoopOne,
    Disabled,
}

/// Zone settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneSettings {
    #[serde(default)]
    pub shuffle: bool,
    #[serde(default)]
    pub auto_radio: bool,
    #[serde(default = "default_loop_mode")]
    pub r#loop: LoopMode,
}

fn default_loop_mode() -> LoopMode {
    LoopMode::Disabled
}

/// Information about the currently playing track.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NowPlaying {
    #[serde(default)]
    pub seek_position: Option<f64>,
    #[serde(default)]
    pub length: Option<f64>,
    #[serde(default)]
    pub image_key: Option<String>,
    pub one_line: LineInfo,
    #[serde(default)]
    pub two_line: Option<TwoLineInfo>,
    #[serde(default)]
    pub three_line: Option<ThreeLineInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineInfo {
    pub line1: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwoLineInfo {
    pub line1: String,
    #[serde(default)]
    pub line2: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreeLineInfo {
    pub line1: String,
    #[serde(default)]
    pub line2: Option<String>,
    #[serde(default)]
    pub line3: Option<String>,
}

/// A Roon playback zone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Zone {
    pub zone_id: String,
    pub display_name: String,
    pub state: PlayState,
    #[serde(default)]
    pub outputs: Vec<Output>,
    #[serde(default)]
    pub now_playing: Option<NowPlaying>,
    #[serde(default)]
    pub settings: Option<ZoneSettings>,
    #[serde(default)]
    pub seek_position: Option<f64>,
    #[serde(default)]
    pub is_previous_allowed: bool,
    #[serde(default)]
    pub is_next_allowed: bool,
    #[serde(default)]
    pub is_pause_allowed: bool,
    #[serde(default)]
    pub is_play_allowed: bool,
    #[serde(default)]
    pub is_seek_allowed: bool,
    #[serde(default)]
    pub queue_items_remaining: Option<u32>,
    #[serde(default)]
    pub queue_time_remaining: Option<f64>,
}

/// Seek position update for a zone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneSeek {
    pub zone_id: String,
    #[serde(default)]
    pub seek_position: Option<f64>,
    #[serde(default)]
    pub queue_time_remaining: Option<f64>,
}
