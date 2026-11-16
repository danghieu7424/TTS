use anyhow::{Context, bail};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use vieneu_tts::{TtsConfig, TtsSynthesizer};

fn main() -> anyhow::Result<()> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest_dir
        .ancestors()
        .nth(2)
        .expect("facade crate is nested in the workspace")
        .to_path_buf();
    let mut args = std::env::args_os().skip(1);
    let mut output_dir = workspace.join("output");
    let mut input_paths = Vec::new();
    while let Some(argument) = args.next() {
        if argument == "--out-dir" {
            let Some(path) = args.next() else {
                bail!("Thiếu đường dẫn sau --out-dir");
            };
            output_dir = PathBuf::from(path);
        } else if argument.to_string_lossy().starts_with("--") {
            bail!(
                "Tham số không được hỗ trợ: {}",
                PathBuf::from(argument).display()
            );
        } else {
            input_paths.push(PathBuf::from(argument));
        }
    }
    if output_dir.is_relative() {
        output_dir = workspace.join(output_dir);
    }
    let input_paths = if input_paths.is_empty() {
        [
            workspace.join("input\\in\\S02E001.merged.zh.vi.vtt"),
            workspace.join("input\\in\\S02E002.merged.zh.vi.vtt"),
            workspace.join("input\\input\\第001集.merged.zh.vi.vtt"),
            workspace.join("input\\input\\第001集.zh.vi.vtt"),
        ]
        .to_vec()
    } else {
        input_paths
    };
    fs::create_dir_all(&output_dir)
        .with_context(|| format!("Không thể tạo thư mục output: {}", output_dir.display()))?;

    let synthesizer = TtsSynthesizer::new(
        TtsConfig::new(workspace.join("Models"))
            .g2p_bridge_path(workspace.join("Models\\g2p_bridge\\g2p_bridge.exe"))
            .num_workers(4),
    );
    let batch_started = Instant::now();
    for path in input_paths {
        let started = Instant::now();
        let result = synthesizer.synthesize_dubbed_audio(&path, "Trúc Ly")?;
        let elapsed = started.elapsed().as_secs_f64();
        let audio_duration =
            result.audio.samples.len() as f64 / f64::from(result.audio.sample_rate);
        let output_path = output_dir.join(output_filename(&path)?);
        fs::write(&output_path, result.audio.wav_bytes()?)
            .with_context(|| format!("Không thể ghi WAV: {}", output_path.display()))?;
        println!(
            "{}\t{}\t{}\t{}\t{elapsed:.3}\t{audio_duration:.3}\t{:.4}\t{}",
            path.display(),
            result.num_cues,
            result.filtered_captions,
            result.cues_read,
            elapsed / audio_duration,
            output_path.display(),
        );
    }
    println!(
        "batch_elapsed_seconds\t{:.3}",
        batch_started.elapsed().as_secs_f64()
    );
    Ok(())
}

fn output_filename(input: &Path) -> anyhow::Result<OsString> {
    let name = input
        .file_name()
        .and_then(|name| name.to_str())
        .context("Tên file input không hợp lệ")?;
    let output_name = match name {
        "S02E001.merged.zh.vi.vtt" => "S02E001.wav".to_string(),
        "S02E002.merged.zh.vi.vtt" => "S02E002.wav".to_string(),
        "第001集.merged.zh.vi.vtt" => "第001集.merged.wav".to_string(),
        "第001集.zh.vi.vtt" => "第001集.wav".to_string(),
        _ => name
            .strip_suffix(".merged.zh.vi.vtt")
            .map(|stem| format!("{stem}.merged.wav"))
            .or_else(|| {
                name.strip_suffix(".zh.vi.vtt")
                    .map(|stem| format!("{stem}.wav"))
            })
            .ok_or_else(|| anyhow::anyhow!("Tên file VTT không đúng quy ước: {name}"))?,
    };
    Ok(OsString::from(output_name))
}
