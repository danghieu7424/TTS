use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};
use vieneu_tts::AudioBuffer;
use vieneu_tts_core::dsp::{
    f32_to_i16_pcm, trim_and_fade, vocal_clarity_filter, wsola_time_stretch,
};
use vieneu_tts_core::normalizer::normalize;
use vieneu_tts_core::vtt::{SubtitleCue, parse_vtt_with_stats};
use vieneu_tts_engine::{G2pWorker, VietNEUTurboEngine, ensure_v3turbo_models};

const SAMPLE_RATE: u32 = 48_000;

struct CueStages {
    cue: SubtitleCue,
    available_sec: f64,
    speed_ratio: f64,
    raw: Vec<f32>,
    after_dsp: Vec<f32>,
    after_wsola: Vec<f32>,
    phonemes: String,
}

fn main() -> Result<()> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest_dir
        .ancestors()
        .nth(2)
        .context("Facade crate is not under a workspace")?;
    let input = workspace.join("input\\in\\S02E001.merged.zh.vi.vtt");
    let output_dir = workspace.join("output\\debug_noise");
    fs::create_dir_all(&output_dir)?;

    let content = fs::read_to_string(&input)
        .with_context(|| format!("Không thể đọc VTT: {}", input.display()))?;
    let parsed = parse_vtt_with_stats(&content)?;
    if parsed.filtered_captions != 6 || parsed.cues.len() != 52 {
        bail!(
            "Kỳ vọng lọc 6 caption và giữ 52 cue, nhận {} và {}",
            parsed.filtered_captions,
            parsed.cues.len()
        );
    }

    let model_root = workspace.join("Models");
    let events = None;
    let model_dir = ensure_v3turbo_models(&model_root, &events)?;
    let mut engine = VietNEUTurboEngine::new(&model_dir, 3, false, &events)?;
    let bridge_path = model_root.join("g2p_bridge\\g2p_bridge.exe");
    let mut g2p = G2pWorker::new(&model_root, Some(&bridge_path), None, None)?;

    let short = render_selected(&mut engine, &mut g2p, &parsed.cues, "Đến đây.")?;
    if short.speed_ratio > 1.03 {
        bail!(
            "Cue ngắn được chọn bất ngờ cần nén {:.3}x",
            short.speed_ratio
        );
    }
    write_stages(&output_dir, &short, "short_")?;
    println!(
        "short\tcue={}\tavailable={:.3}s\traw={:.3}s\tdsp={:.3}s\trequested_ratio={:.3}\tpeak_raw={:.5}\tpeak_dsp={:.5}",
        short.cue.index,
        short.available_sec,
        short.raw.len() as f64 / f64::from(SAMPLE_RATE),
        short.after_dsp.len() as f64 / f64::from(SAMPLE_RATE),
        short.speed_ratio,
        peak(&short.raw),
        peak(&short.after_dsp),
    );

    let long = render_selected(
        &mut engine,
        &mut g2p,
        &parsed.cues,
        "Ta quyết định sẽ trở thành thú hợp đồng",
    )?;
    if long.speed_ratio <= 1.03 {
        bail!(
            "Cue dài được chọn không cần WSOLA (ratio {:.3}x); chọn cue khác",
            long.speed_ratio
        );
    }
    write_stages(&output_dir, &long, "")?;
    println!(
        "long\tcue={}\tavailable={:.3}s\traw={:.3}s\tdsp={:.3}s\twsola={:.3}s\trequested_ratio={:.3}\tactual_ratio={:.3}\tpeak_raw={:.5}\tpeak_dsp={:.5}\tpeak_wsola={:.5}",
        long.cue.index,
        long.available_sec,
        long.raw.len() as f64 / f64::from(SAMPLE_RATE),
        long.after_dsp.len() as f64 / f64::from(SAMPLE_RATE),
        long.after_wsola.len() as f64 / f64::from(SAMPLE_RATE),
        long.speed_ratio,
        long.after_dsp.len() as f64 / long.after_wsola.len() as f64,
        peak(&long.raw),
        peak(&long.after_dsp),
        peak(&long.after_wsola),
    );

    for (name, voice) in [("anh_khoi", "Anh Khôi"), ("mai_anh", "Mai Anh")] {
        if !engine.voices.contains_key(voice) {
            bail!("Không tìm thấy voice preset: {voice}");
        }
        let raw = engine.synthesize(&short.phonemes, voice)?;
        let trimmed = trim_and_fade(&raw, SAMPLE_RATE as usize, -45.0, 0.03, 0.015);
        let after_dsp = vocal_clarity_filter(&trimmed, SAMPLE_RATE as usize, 60.0, 22_000.0, 1.0);
        write_wav(
            &output_dir.join(format!("voice_{name}.wav")),
            &f32_to_i16_pcm(&after_dsp),
        )?;
        println!(
            "voice\t{name}\tpeak_raw={:.5}\tpeak_dsp={:.5}",
            peak(&raw),
            peak(&after_dsp),
        );
    }

    println!(
        "captions_filtered={}\tcues_retained={}",
        parsed.filtered_captions,
        parsed.cues.len()
    );
    Ok(())
}

fn render_selected(
    engine: &mut VietNEUTurboEngine,
    g2p: &mut G2pWorker,
    cues: &[SubtitleCue],
    text_prefix: &str,
) -> Result<CueStages> {
    let (ordinal, cue) = cues
        .iter()
        .enumerate()
        .find(|(_, cue)| cue.text.starts_with(text_prefix))
        .with_context(|| format!("Không tìm thấy cue bắt đầu bằng {text_prefix:?}"))?;
    let text = text_with_punctuation(cue, cues.get(ordinal + 1));
    let phonemes = g2p.phonemize(&normalize(&text))?;
    let raw = engine.synthesize(&phonemes, "Trúc Ly")?;
    let after_dsp = vocal_clarity_filter(
        &trim_and_fade(&raw, SAMPLE_RATE as usize, -45.0, 0.03, 0.015),
        SAMPLE_RATE as usize,
        60.0,
        22_000.0,
        1.0,
    );
    let next_start = cues
        .get(ordinal + 1)
        .map(|next| next.start_sec - 0.06)
        .unwrap_or(cue.end_sec + 2.0);
    let available_sec = (next_start - cue.start_sec).max(cue.end_sec - cue.start_sec);
    let speed_ratio = after_dsp.len() as f64 / f64::from(SAMPLE_RATE) / available_sec;
    let after_wsola = if speed_ratio > 1.0 && available_sec > 0.1 {
        let target_len = (available_sec * f64::from(SAMPLE_RATE)).round() as usize;
        wsola_time_stretch(&after_dsp, target_len, SAMPLE_RATE as usize)
    } else {
        after_dsp.clone()
    };

    let stages = CueStages {
        cue: cue.clone(),
        available_sec,
        speed_ratio,
        raw,
        after_dsp,
        after_wsola,
        phonemes,
    };
    Ok(stages)
}

fn write_stages(output_dir: &Path, stages: &CueStages, prefix: &str) -> Result<()> {
    fs::create_dir_all(output_dir)?;
    write_wav(
        &output_dir.join(format!("{prefix}01_raw_engine.wav")),
        &f32_to_i16_pcm(&stages.raw),
    )?;
    write_wav(
        &output_dir.join(format!("{prefix}02_after_dsp.wav")),
        &f32_to_i16_pcm(&stages.after_dsp),
    )?;
    write_wav(
        &output_dir.join(format!("{prefix}03_after_wsola.wav")),
        &f32_to_i16_pcm(&stages.after_wsola),
    )?;
    Ok(())
}

fn write_wav(path: &Path, samples: &[i16]) -> Result<()> {
    let audio = AudioBuffer {
        samples: samples.to_vec(),
        sample_rate: SAMPLE_RATE,
    };
    fs::write(path, audio.wav_bytes()?)
        .with_context(|| format!("Không thể ghi WAV: {}", path.display()))
}

fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()))
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
