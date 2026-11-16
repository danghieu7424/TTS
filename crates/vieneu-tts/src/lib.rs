use std::path::PathBuf;
use vieneu_tts_core::events::EventCallback;

#[cfg(feature = "turbo")]
mod pipeline;

#[cfg(feature = "turbo")]
pub use pipeline::{AudioBuffer, TTSOutput, TtsSynthesizer};
pub use vieneu_tts_core::events::TtsEvent;
pub use vieneu_tts_core::{dsp, normalizer, vtt};

#[derive(Clone)]
pub struct TtsConfig {
    pub model_dir: PathBuf,
    pub threads: usize,
    pub num_workers: usize,
    pub use_gpu: bool,
    pub g2p_bridge_path: Option<PathBuf>,
    pub python_path: Option<PathBuf>,
    pub event_callback: Option<EventCallback>,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            model_dir: PathBuf::new(),
            threads: std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1),
            num_workers: 4,
            use_gpu: false,
            g2p_bridge_path: None,
            python_path: None,
            event_callback: None,
        }
    }
}

impl TtsConfig {
    pub fn new(model_dir: impl Into<PathBuf>) -> Self {
        Self {
            model_dir: model_dir.into(),
            ..Self::default()
        }
    }

    pub fn on_event(mut self, callback: EventCallback) -> Self {
        self.event_callback = Some(callback);
        self
    }

    pub fn use_gpu(mut self, enabled: bool) -> Self {
        self.use_gpu = enabled;
        self
    }

    pub fn threads(mut self, threads: usize) -> Self {
        self.threads = threads.max(1);
        self
    }

    pub fn num_workers(mut self, workers: usize) -> Self {
        self.num_workers = workers.max(1);
        self
    }

    pub fn g2p_bridge_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.g2p_bridge_path = Some(path.into());
        self
    }

    pub fn python_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.python_path = Some(path.into());
        self
    }
}
