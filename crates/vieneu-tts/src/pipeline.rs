use anyhow::{Context, Result, anyhow, bail};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use vieneu_tts_core::dsp::{
    f32_to_i16_pcm, trim_and_fade, vocal_clarity_filter, wsola_time_stretch,
};
use vieneu_tts_core::events::{TtsEvent, emit};
use vieneu_tts_core::normalizer::normalize;
use vieneu_tts_core::vtt::{SubtitleCue, parse_vtt_with_stats};
use vieneu_tts_engine::{G2pWorker, VietNEUTurboEngine, ensure_v3turbo_models};

use crate::TtsConfig;

const SAMPLE_RATE: u32 = 48_000;

#[derive(Clone, Debug, PartialEq)]
pub struct AudioBuffer {
    pub samples: Vec<i16>,
    pub sample_rate: u32,
}

impl AudioBuffer {
    pub fn wav_bytes(&self) -> Result<Vec<u8>> {
        let data_len = self
            .samples
            .len()
            .checked_mul(2)
            .ok_or_else(|| anyhow!("AudioBuffer quá lớn để mã hóa WAV"))?;
        let data_len = u32::try_from(data_len)
            .map_err(|_| anyhow!("AudioBuffer vượt quá giới hạn kích thước WAV"))?;
        let riff_len = data_len
            .checked_add(36)
            .ok_or_else(|| anyhow!("AudioBuffer vượt quá giới hạn kích thước WAV"))?;
        let byte_rate = self
            .sample_rate
            .checked_mul(2)
            .ok_or_else(|| anyhow!("Sample rate vượt quá giới hạn WAV"))?;

        let capacity = usize::try_from(data_len)
            .ok()
            .and_then(|length| length.checked_add(44))
            .ok_or_else(|| anyhow!("AudioBuffer quá lớn để mã hóa WAV"))?;
        let mut bytes = Vec::with_capacity(capacity);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&riff_len.to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&self.sample_rate.to_le_bytes());
        bytes.extend_from_slice(&byte_rate.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for sample in &self.samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        Ok(bytes)
    }
}

#[derive(Debug, Clone)]
pub struct TTSOutput {
    pub audio: AudioBuffer,
    pub num_cues: usize,
    pub filtered_captions: usize,
    pub cues_read: usize,
    pub voice: String,
}

struct CueTask {
    ordinal: usize,
    text: String,
}

struct RenderedCue {
    ordinal: usize,
    samples: Vec<f32>,
}

type CueQueue = Arc<Mutex<VecDeque<CueTask>>>;

struct WorkerContext {
    worker_id: usize,
    queue: CueQueue,
    cancelled: Arc<AtomicBool>,
    sender: mpsc::Sender<Result<RenderedCue>>,
    model_dir: std::path::PathBuf,
    config: TtsConfig,
    intra_threads: usize,
    voice: String,
}

pub struct TtsSynthesizer {
    config: TtsConfig,
}

impl TtsSynthesizer {
    pub fn new(config: TtsConfig) -> Self {
        Self { config }
    }

    pub fn synthesize_dubbed_audio(&self, vi_vtt: &Path, voice: &str) -> Result<TTSOutput> {
        let content = std::fs::read_to_string(vi_vtt)
            .with_context(|| format!("Không thể đọc file phụ đề: {}", vi_vtt.display()))?;
        let parsed = parse_vtt_with_stats(&content)?;
        if parsed.cues.is_empty() {
            bail!("File phụ đề không chứa câu thoại nào: {}", vi_vtt.display());
        }
        if parsed.malformed_line_settings > 0 {
            emit(
                &self.config.event_callback,
                TtsEvent::Warning(format!(
                    "Giữ lại {} cue có setting line: sai cú pháp.",
                    parsed.malformed_line_settings
                )),
            );
        }

        let mut tasks = Vec::new();
        for (ordinal, cue) in parsed.cues.iter().enumerate() {
            let text = remove_cjk(&text_with_punctuation(cue, parsed.cues.get(ordinal + 1)));
            if text.chars().any(char::is_alphanumeric) {
                tasks.push(CueTask { ordinal, text });
            }
        }
        if tasks.is_empty() {
            bail!("Không còn nội dung tiếng Việt hoặc Latin để đọc sau khi lọc chữ Hán.");
        }

        emit(
            &self.config.event_callback,
            TtsEvent::Log(format!(
                "Đã nạp {} cue; lọc {} caption line < 50%; sẽ đọc {} cue.",
                parsed.total_cues,
                parsed.filtered_captions,
                tasks.len()
            )),
        );

        let total_tasks = tasks.len();
        let model_dir = ensure_v3turbo_models(&self.config.model_dir, &self.config.event_callback)?;
        let worker_count = self.config.num_workers.max(1).min(total_tasks);
        let intra_threads = (self.config.threads.max(1) / worker_count).max(1);
        let queue = Arc::new(Mutex::new(VecDeque::from(tasks)));
        let cancelled = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let mut workers = Vec::with_capacity(worker_count);

        for worker_id in 0..worker_count {
            let queue = Arc::clone(&queue);
            let cancelled = Arc::clone(&cancelled);
            let sender = sender.clone();
            let model_dir = model_dir.clone();
            let config = self.config.clone();
            let voice = voice.to_string();
            workers.push(thread::spawn(move || {
                let result = run_worker(WorkerContext {
                    worker_id,
                    queue,
                    cancelled: Arc::clone(&cancelled),
                    sender: sender.clone(),
                    model_dir,
                    config,
                    intra_threads,
                    voice,
                });
                if let Err(error) = result {
                    cancelled.store(true, Ordering::Release);
                    let _ = sender.send(Err(error));
                }
            }));
        }
        drop(sender);

        let mut rendered: Vec<Option<Vec<f32>>> = vec![None; parsed.cues.len()];
        let mut first_error = None;
        let mut completed = 0;
        for message in receiver {
            match message {
                Ok(cue) => {
                    completed += 1;
                    rendered[cue.ordinal] = Some(cue.samples);
                    emit(
                        &self.config.event_callback,
                        TtsEvent::Progress {
                            stage: "synthesis".to_string(),
                            current: completed,
                            total: total_tasks,
                            message: format!("Đã tổng hợp {completed}/{total_tasks} cue."),
                        },
                    );
                }
                Err(error) => {
                    cancelled.store(true, Ordering::Release);
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        for worker in workers {
            if worker.join().is_err() && first_error.is_none() {
                first_error = Some(anyhow!("Worker tổng hợp bị panic"));
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }

        let last_end = parsed
            .cues
            .last()
            .map(|cue| cue.end_sec)
            .unwrap_or_default();
        let initial_samples = ((last_end + 3.0) * f64::from(SAMPLE_RATE)) as usize;
        let mut timeline = vec![0.0f32; initial_samples];
        let mut current_end = 0.0f64;
        for (ordinal, cue) in parsed.cues.iter().enumerate() {
            emit(
                &self.config.event_callback,
                TtsEvent::Progress {
                    stage: "alignment".to_string(),
                    current: ordinal + 1,
                    total: parsed.cues.len(),
                    message: format!("Căn chỉnh cue #{}", cue.index),
                },
            );
            let Some(samples) = rendered[ordinal].as_ref() else {
                continue;
            };
            if samples.is_empty() {
                continue;
            }

            let next_start = parsed
                .cues
                .get(ordinal + 1)
                .map(|next| next.start_sec - 0.06)
                .unwrap_or(cue.end_sec + 2.0);
            let available = (next_start - cue.start_sec).max(cue.end_sec - cue.start_sec);
            let stretched =
                if samples.len() as f64 / f64::from(SAMPLE_RATE) > available && available > 0.1 {
                    let target_len = (available * f64::from(SAMPLE_RATE)).round() as usize;
                    wsola_time_stretch(samples, target_len, SAMPLE_RATE as usize)
                } else {
                    samples.clone()
                };

            let start = cue.start_sec.max(current_end);
            let start_sample = (start * f64::from(SAMPLE_RATE)) as usize;
            let required = start_sample.saturating_add(stretched.len());
            if required > timeline.len() {
                timeline.resize(required.saturating_add(SAMPLE_RATE as usize * 2), 0.0);
            }
            timeline[start_sample..start_sample + stretched.len()].copy_from_slice(&stretched);
            current_end = start + stretched.len() as f64 / f64::from(SAMPLE_RATE) + 0.04;
        }

        Ok(TTSOutput {
            audio: AudioBuffer {
                samples: f32_to_i16_pcm(&timeline),
                sample_rate: SAMPLE_RATE,
            },
            num_cues: parsed.total_cues,
            filtered_captions: parsed.filtered_captions,
            cues_read: completed,
            voice: voice.to_string(),
        })
    }
}

fn run_worker(context: WorkerContext) -> Result<()> {
    let WorkerContext {
        worker_id,
        queue,
        cancelled,
        sender,
        model_dir,
        config,
        intra_threads,
        voice,
    } = context;
    let mut engine = VietNEUTurboEngine::new(
        &model_dir,
        intra_threads,
        config.use_gpu,
        &config.event_callback,
    )
    .with_context(|| format!("Không thể khởi tạo engine cho worker #{worker_id}"))?;
    let mut g2p = G2pWorker::new(
        &config.model_dir,
        config.g2p_bridge_path.as_deref(),
        config.python_path.as_deref(),
        config.event_callback.clone(),
    )
    .with_context(|| format!("Không thể khởi tạo G2P cho worker #{worker_id}"))?;

    while !cancelled.load(Ordering::Acquire) {
        let task = queue
            .lock()
            .map_err(|_| anyhow!("CueQueue bị poison"))?
            .pop_front();
        let Some(task) = task else {
            break;
        };
        let phonemes = g2p.phonemize(&normalize(&task.text))?;
        let audio = engine.synthesize(&phonemes, &voice)?;
        let trimmed = trim_and_fade(&audio, SAMPLE_RATE as usize, -45.0, 0.03, 0.015);
        let samples = vocal_clarity_filter(&trimmed, SAMPLE_RATE as usize, 60.0, 22_000.0, 1.0);
        sender
            .send(Ok(RenderedCue {
                ordinal: task.ordinal,
                samples,
            }))
            .map_err(|_| anyhow!("Không thể gửi kết quả từ worker #{worker_id}"))?;
    }
    Ok(())
}

fn text_with_punctuation(cue: &SubtitleCue, next: Option<&SubtitleCue>) -> String {
    let text = cue.text.trim().trim_start_matches(['-', '—', '–']).trim();
    if text.is_empty() || text.ends_with(['.', '!', '?', '…', ',', ';', ':']) {
        return text.to_string();
    }
    let continuation = next.is_some_and(|next| {
        next.text
            .trim()
            .chars()
            .next()
            .is_some_and(char::is_lowercase)
            || next.start_sec - cue.end_sec < 0.6
    });
    format!("{text}{}", if continuation { ',' } else { '.' })
}

fn remove_cjk(text: &str) -> String {
    let mut characters = text.chars().peekable();
    let mut filtered = String::new();
    while let Some(character) = characters.next() {
        if is_cjk_ideograph(character) {
            let previous_is_word = filtered
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric);
            while characters.peek().copied().is_some_and(is_cjk_ideograph) {
                let _ = characters.next();
            }
            let next_is_word = characters
                .peek()
                .copied()
                .is_some_and(char::is_alphanumeric);
            if previous_is_word && next_is_word && !filtered.ends_with(' ') {
                filtered.push(' ');
            }
        } else {
            filtered.push(character);
        }
    }
    filtered.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_cjk_ideograph(character: char) -> bool {
    matches!(
        character as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x20000..=0x2FA1F
            | 0x30000..=0x323AF
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_han_characters_without_changing_latin_or_vietnamese_text() {
        assert_eq!(remove_cjk("你好 Xin chào世界!"), "Xin chào!");
        assert_eq!(remove_cjk("hello世界world"), "hello world");
        assert_eq!(remove_cjk("純中文"), "");
    }

    #[test]
    fn serializes_mono_pcm16_to_wav_in_memory() {
        let audio = AudioBuffer {
            samples: vec![0x1234, -2],
            sample_rate: SAMPLE_RATE,
        };
        let bytes = audio.wav_bytes().unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[40..44], &4u32.to_le_bytes());
        assert_eq!(&bytes[44..], &[0x34, 0x12, 0xfe, 0xff]);
    }
}
