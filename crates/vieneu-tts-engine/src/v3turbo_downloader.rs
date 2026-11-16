/****
 * [ATOM]: v3turbo_downloader
 * Chức năng: Quản lý cấp phát và đồng bộ tài nguyên mô hình VieNeu-TTS v3-Turbo (ONNX INT8)
 * và MOSS Audio Tokenizer Decoder (48kHz) trên máy cục bộ hoặc từ Hugging Face.
 * Ranh giới bảo vệ: Tự động kiểm tra cache Hugging Face trước khi tải mới để tiết kiệm băng thông.
 ****/
use crate::log_event;
use anyhow::{Result, anyhow};
use hf_hub::api::sync::ApiBuilder;
use std::fs;
use std::path::{Path, PathBuf};
use vieneu_tts_core::events::EventCallback;

pub const V3TURBO_HF_REPO: &str = "pnnbao-ump/VieNeu-TTS-v3-Turbo";
pub const MOSS_CODEC_HF_REPO: &str = "OpenMOSS-Team/MOSS-Audio-Tokenizer-Nano-ONNX";

/// Danh sách các file cốt lõi trong subfolder onnx_int8 của VieNeu-TTS-v3-Turbo
pub const V3TURBO_GRAPH_FILES: &[&str] = &[
    "config.json",
    "tokenizer.json",
    "vieneu_v3_heads.npz",
    "vieneu_prefill.onnx",
    "vieneu_decode_step.onnx",
    "vieneu_acoustic_cached.onnx",
    "vieneu_backbone_shared.data",
];

/// Danh sách các file của MOSS Audio Codec Decoder (48kHz)
pub const MOSS_CODEC_FILES: &[&str] = &[
    "moss_audio_tokenizer_decode_full.onnx",
    "moss_audio_tokenizer_decode_shared.data",
];

/// Đảm bảo toàn bộ tài nguyên của VieNeu-TTS v3 Turbo INT8 sẵn sàng trong `model_dir/tts/v3turbo`
pub fn ensure_v3turbo_models(model_dir: &Path, events: &Option<EventCallback>) -> Result<PathBuf> {
    let target_dir = model_dir.join("tts").join("v3turbo");
    fs::create_dir_all(&target_dir)?;

    let mut all_files: Vec<&str> = Vec::new();
    all_files.extend_from_slice(V3TURBO_GRAPH_FILES);
    all_files.extend_from_slice(MOSS_CODEC_FILES);
    all_files.push("voices_v3_turbo.json");

    let is_complete = all_files.iter().all(|file| target_dir.join(file).exists());
    if is_complete {
        return Ok(target_dir);
    }

    log_event(
        events,
        "Kiểm tra tài nguyên VieNeu-TTS v3 Turbo (ONNX INT8).",
    );

    // 1. Kiểm tra copy file voices_v3_turbo.json từ repo nội bộ nếu chưa có
    let voices_dest = target_dir.join("voices_v3_turbo.json");
    if !voices_dest.exists() {
        let local_voices_candidates = [
            model_dir.join("v3turbo").join("voices_v3_turbo.json"),
            model_dir.join("voices_v3_turbo.json"),
        ];
        let mut copied = false;
        for cand in &local_voices_candidates {
            if cand.exists() {
                fs::copy(cand, &voices_dest)?;
                copied = true;
                break;
            }
        }
        if !copied {
            const EMBEDDED_VOICES: &str = include_str!("../assets/voices_v3_turbo.json");
            fs::write(&voices_dest, EMBEDDED_VOICES)?;
        }
    }

    let local_model_dir = model_dir.join("v3turbo");
    if local_model_dir.is_dir() {
        for file in all_files
            .iter()
            .copied()
            .filter(|file| *file != "voices_v3_turbo.json")
        {
            let source = local_model_dir.join(file);
            let destination = target_dir.join(file);
            if !destination.exists() && source.is_file() {
                fs::copy(source, destination)?;
            }
        }
    }

    // Hugging Face Hub resolves its own cache before downloading.
    let still_missing: Vec<&&str> = all_files
        .iter()
        .filter(|&&f| !target_dir.join(f).exists() && f != "voices_v3_turbo.json")
        .collect();

    if !still_missing.is_empty() {
        log_event(
            events,
            format!(
                "Tải {} file còn thiếu từ Hugging Face.",
                still_missing.len()
            ),
        );
        let api = ApiBuilder::new()
            .with_progress(false)
            .build()
            .map_err(|e| anyhow!("Không thể khởi tạo HF API: {}", e))?;

        let v3_repo = api.model(V3TURBO_HF_REPO.to_string());
        for &f in V3TURBO_GRAPH_FILES {
            let dest = target_dir.join(f);
            if !dest.exists() {
                // Trên HF, các file này nằm ở subfolder onnx_int8
                let remote_rel = format!("onnx_int8/{}", f);
                match v3_repo.get(&remote_rel) {
                    Ok(downloaded) => {
                        fs::copy(downloaded, &dest)?;
                    }
                    Err(e) => {
                        return Err(anyhow!(
                            "Lỗi tải {} từ {}: {:?}",
                            remote_rel,
                            V3TURBO_HF_REPO,
                            e
                        ));
                    }
                }
            }
        }

        let moss_repo = api.model(MOSS_CODEC_HF_REPO.to_string());
        for &f in MOSS_CODEC_FILES {
            let dest = target_dir.join(f);
            if !dest.exists() {
                match moss_repo.get(f) {
                    Ok(downloaded) => {
                        fs::copy(downloaded, &dest)?;
                    }
                    Err(e) => {
                        return Err(anyhow!("Lỗi tải {} từ {}: {:?}", f, MOSS_CODEC_HF_REPO, e));
                    }
                }
            }
        }
    }

    log_event(events, "VieNeu-TTS v3 Turbo INT8 đã sẵn sàng.");
    Ok(target_dir)
}
