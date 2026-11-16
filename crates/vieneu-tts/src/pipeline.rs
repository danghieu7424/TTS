use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use vieneu_tts_core::dsp::{f32_to_i16_pcm, trim_and_fade, vocal_clarity_filter};
use vieneu_tts_core::events::{TtsEvent, emit};
use vieneu_tts_core::normalizer::normalize;
use vieneu_tts_core::vtt::{SubtitleCue, parse_vtt};
use vieneu_tts_engine::{G2pWorker, VietNEUTurboEngine, ensure_v3turbo_models};

use crate::TtsConfig;

const SAMPLE_RATE: u32 = 48_000;

#[derive(Debug, Clone)]
pub struct TTSOutput {
    pub dubbed_audio: PathBuf,
    pub dubbed_audio_raw: PathBuf,
    pub num_cues: usize,
    pub voice: String,
}

pub struct TtsSynthesizer {
    config: TtsConfig,
}

impl TtsSynthesizer {
    pub fn new(config: TtsConfig) -> Self {
        Self { config }
    }

    pub fn synthesize_dubbed_audio(
        &self,
        vi_vtt: &Path,
        tmp_dir: &Path,
        voice: &str,
        force_tts: bool,
    ) -> Result<TTSOutput> {
        let content = fs::read_to_string(vi_vtt)
            .with_context(|| format!("Không thể đọc file phụ đề: {}", vi_vtt.display()))?;
        let cues = parse_vtt(&content)?;
        if cues.is_empty() {
            bail!("File phụ đề không chứa câu thoại nào: {}", vi_vtt.display());
        }

        emit(
            &self.config.event_callback,
            TtsEvent::Log(format!("Đã nạp {} câu phụ đề.", cues.len())),
        );
        let model_dir = ensure_v3turbo_models(&self.config.model_dir, &self.config.event_callback)?;
        let mut engine = VietNEUTurboEngine::new(
            &model_dir,
            self.config.threads.max(1),
            self.config.use_gpu,
            &self.config.event_callback,
        )?;
        let mut g2p = G2pWorker::new(
            &self.config.model_dir,
            self.config.g2p_bridge_path.as_deref(),
            self.config.python_path.as_deref(),
            self.config.event_callback.clone(),
        )?;

        let tts_dir = tmp_dir.join("tts");
        fs::create_dir_all(&tts_dir)?;
        if force_tts {
            for entry in fs::read_dir(&tts_dir)? {
                let path = entry?.path();
                if path.extension().is_some_and(|extension| extension == "wav") {
                    fs::remove_file(path)?;
                }
            }
        }

        let total = cues.len();
        for (index, cue) in cues.iter().enumerate() {
            emit(
                &self.config.event_callback,
                TtsEvent::Progress {
                    stage: "synthesis".to_string(),
                    current: index + 1,
                    total,
                    message: format!("Tổng hợp cue #{}", cue.index),
                },
            );
            let output = tts_dir.join(format!("{}.wav", cue.index));
            if output.exists() && output.metadata()?.len() > 100 {
                continue;
            }

            let text = text_with_punctuation(cue, cues.get(index + 1));
            let phonemes = g2p.phonemize(&normalize(&text))?;
            let audio = engine.synthesize(&phonemes, voice)?;
            let trimmed = trim_and_fade(&audio, SAMPLE_RATE as usize, -45.0, 0.03, 0.015);
            let filtered =
                vocal_clarity_filter(&trimmed, SAMPLE_RATE as usize, 60.0, 22_000.0, 1.0);
            write_wav(&output, &f32_to_i16_pcm(&filtered))?;
        }

        let last_end = cues.last().map(|cue| cue.end_sec).unwrap_or_default();
        let initial_samples = ((last_end + 3.0) * SAMPLE_RATE as f64) as usize;
        let mut timeline = vec![0i16; initial_samples];
        let mut current_end = 0.0f64;
        for (index, cue) in cues.iter().enumerate() {
            emit(
                &self.config.event_callback,
                TtsEvent::Progress {
                    stage: "alignment".to_string(),
                    current: index + 1,
                    total,
                    message: format!("Căn chỉnh cue #{}", cue.index),
                },
            );

            let wav_path = tts_dir.join(format!("{}.wav", cue.index));
            if !wav_path.exists() {
                continue;
            }
            let samples = read_wav(&wav_path)?;
            if samples.is_empty() {
                continue;
            }

            let next_start = cues
                .get(index + 1)
                .map(|next| next.start_sec - 0.06)
                .unwrap_or(cue.end_sec + 2.0);
            let available = (next_start - cue.start_sec).max(cue.end_sec - cue.start_sec);
            let samples =
                if samples.len() as f64 / SAMPLE_RATE as f64 > available && available > 0.1 {
                    self.stretch(&tts_dir, cue.index, &samples, available)?
                } else {
                    samples
                };

            let start = cue.start_sec.max(current_end);
            let start_sample = (start * SAMPLE_RATE as f64) as usize;
            let required = start_sample.saturating_add(samples.len());
            if required > timeline.len() {
                timeline.resize(required.saturating_add(SAMPLE_RATE as usize * 2), 0);
            }
            for (offset, sample) in samples.iter().enumerate() {
                timeline[start_sample + offset] = *sample;
            }
            current_end = start + samples.len() as f64 / SAMPLE_RATE as f64 + 0.04;
        }

        let audio_dir = tmp_dir.join("audio");
        fs::create_dir_all(&audio_dir)?;
        let dubbed_audio_raw = audio_dir.join("dubbed_audio_raw.wav");
        write_wav(&dubbed_audio_raw, &timeline)?;
        let dubbed_audio = audio_dir.join("dubbed_audio.wav");
        fs::copy(&dubbed_audio_raw, &dubbed_audio)?;

        Ok(TTSOutput {
            dubbed_audio,
            dubbed_audio_raw,
            num_cues: cues.len(),
            voice: voice.to_string(),
        })
    }

    fn stretch(
        &self,
        tts_dir: &Path,
        cue_index: usize,
        samples: &[i16],
        target_duration: f64,
    ) -> Result<Vec<i16>> {
        #[cfg(feature = "ffmpeg")]
        {
            let input = tts_dir.join(format!("{cue_index}_trimmed.wav"));
            let output = tts_dir.join(format!("{cue_index}_stretched.wav"));
            write_wav(&input, samples)?;
            let ratio =
                (samples.len() as f64 / SAMPLE_RATE as f64 / target_duration).clamp(1.0, 1.35);
            let ffmpeg = self
                .config
                .ffmpeg_path
                .as_deref()
                .unwrap_or_else(|| Path::new("ffmpeg"));
            let status = Command::new(ffmpeg)
                .args(["-y", "-i"])
                .arg(&input)
                .args(["-filter:a", &format!("atempo={ratio:.3}"), "-ar", "48000"])
                .arg(&output)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .with_context(|| format!("Không thể chạy FFmpeg tại {}", ffmpeg.display()))?;
            if !status.success() {
                bail!("FFmpeg atempo thất bại với cue #{cue_index}");
            }
            let stretched = read_wav(&output);
            let _ = fs::remove_file(input);
            let _ = fs::remove_file(output);
            stretched
        }
        #[cfg(not(feature = "ffmpeg"))]
        {
            let _ = (tts_dir, cue_index, target_duration);
            Ok(samples.to_vec())
        }
    }
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

fn write_wav(path: &Path, samples: &[i16]) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for sample in samples {
        writer.write_sample(*sample)?;
    }
    writer.finalize()?;
    Ok(())
}

fn read_wav(path: &Path) -> Result<Vec<i16>> {
    let reader = hound::WavReader::open(path)?;
    reader
        .into_samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}
