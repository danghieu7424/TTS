use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum TtsEvent {
    Log(String),
    Warning(String),
    Progress {
        stage: String,
        current: usize,
        total: usize,
        message: String,
    },
}

pub type EventCallback = Arc<dyn Fn(TtsEvent) + Send + Sync + 'static>;

pub fn emit(callback: &Option<EventCallback>, event: TtsEvent) {
    if let Some(callback) = callback {
        callback(event);
    }
}
