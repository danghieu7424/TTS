#[cfg(feature = "turbo")]
use vieneu_tts_core::events::{EventCallback, TtsEvent};

#[cfg(feature = "turbo")]
pub(crate) fn log_event(events: &Option<EventCallback>, message: impl Into<String>) {
    if let Some(callback) = events {
        callback(TtsEvent::Log(message.into()));
    }
}

#[cfg(feature = "turbo")]
mod g2p_worker;
#[cfg(feature = "turbo")]
mod v3turbo_downloader;
#[cfg(feature = "turbo")]
mod v3turbo_engine;

#[cfg(feature = "turbo")]
pub use g2p_worker::G2pWorker;
#[cfg(feature = "turbo")]
pub use v3turbo_downloader::ensure_v3turbo_models;
#[cfg(feature = "turbo")]
pub use v3turbo_engine::VietNEUTurboEngine;

#[cfg(feature = "nano")]
pub mod nano;
