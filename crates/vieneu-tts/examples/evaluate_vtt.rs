use std::path::PathBuf;
use std::time::Instant;
use vieneu_tts::{TtsConfig, TtsSynthesizer};

fn main() -> anyhow::Result<()> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest_dir
        .ancestors()
        .nth(2)
        .expect("facade crate is nested in the workspace")
        .to_path_buf();
    let input_paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let input_paths = if input_paths.is_empty() {
        [
            workspace.join("input/in/S02E001.merged.zh.vi.vtt"),
            workspace.join("input/in/S02E002.merged.zh.vi.vtt"),
            workspace.join("input/input/第001集.merged.zh.vi.vtt"),
            workspace.join("input/input/第001集.zh.vi.vtt"),
        ]
        .to_vec()
    } else {
        input_paths
    };

    let synthesizer = TtsSynthesizer::new(
        TtsConfig::new(workspace.join("Models"))
            .g2p_bridge_path(workspace.join("Models/g2p_bridge/g2p_bridge.exe"))
            .num_workers(4),
    );
    for path in input_paths {
        let started = Instant::now();
        let result = synthesizer.synthesize_dubbed_audio(&path, "Trúc Ly")?;
        let elapsed = started.elapsed().as_secs_f64();
        let audio_duration =
            result.audio.samples.len() as f64 / f64::from(result.audio.sample_rate);
        println!(
            "{}\t{}\t{}\t{}\t{elapsed:.3}\t{audio_duration:.3}\t{:.4}",
            path.display(),
            result.num_cues,
            result.filtered_captions,
            result.cues_read,
            elapsed / audio_duration,
        );
    }
    Ok(())
}
