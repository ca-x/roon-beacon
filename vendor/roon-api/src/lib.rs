pub mod browse;
mod client;
pub mod connection;
pub mod core;
mod error;
mod event;
pub mod image;
pub mod output;
pub mod pairing;
pub(crate) mod registry;
pub mod source_control;
pub mod status;
pub mod token;
pub mod transport;
pub mod volume_control;
pub mod zone;

pub use self::core::Core;
pub use browse::{
    Browse, BrowseItem, BrowseList, BrowseOptions, BrowseResult, InputPrompt, LoadOptions,
    LoadResult,
};
pub use client::{RoonClient, RoonClientBuilder};
pub use connection::ConnectionState;
pub use error::ApiError;
pub use event::RoonEvent;
pub use image::{ImageOptions, ImageService};
pub use output::{Output, SourceControl, Volume};
pub use pairing::PairingState;
pub use source_control::{SourceControlDef, SourceControlService, SourceRequest};
pub use status::StatusService;
pub use token::{
    FileStateStore, FileTokenStore, MemoryStateStore, MemoryTokenStore, StateStore, TokenStore,
};
pub use transport::{
    ControlAction, MuteAction, OutputEvent, SeekMode, Transport, VolumeMode, ZoneEvent,
};
pub use volume_control::{VolumeControlDef, VolumeControlService, VolumeRequest};
pub use zone::{NowPlaying, PlayState, Zone, ZoneSeek, ZoneSettings};
