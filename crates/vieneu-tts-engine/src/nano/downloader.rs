/****
 * [ATOM]: downloader
 * Chức năng: Quản lý cấp phát mô hình VieNeu-TTS v3 Nano trên máy trắng
 * Tự động tìm kiếm local cache hoặc tải trực tiếp từ Hugging Face
 ****/
use anyhow::{Result, anyhow};
use hf_hub::api::sync::Api;
use std::fs;
use std::path::{Path, PathBuf};

pub const NANO_HF_REPO: &str = "pnnbao-ump/VieNeu-TTS-v3-Nano";

pub const REQUIRED_FILES: &[&str] = &[
    "config.json",
    "constants.npz",
    "voices.json",
    "text_encoder.onnx",
    "duration_predictor.onnx",
    "vector_estimator.onnx",
    "codec_decoder.onnx",
];

/// Đảm bảo tất cả 7 file mô hình của VieNeu-TTS v3 Nano tồn tại trong `target_dir`
pub fn ensure_v3nano_models(model_dir: &Path) -> Result<PathBuf> {
    let target_dir = model_dir.join("tts").join("v3nano");
    fs::create_dir_all(&target_dir)?;

    let is_complete = REQUIRED_FILES
        .iter()
        .all(|file| target_dir.join(file).exists());
    if is_complete {
        return Ok(target_dir);
    }

    let mut local_search_paths = vec![
        model_dir.join("v3nano"),
        model_dir.join("tts").join("v3nano"),
    ];

    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        local_search_paths.push(parent.join("models").join("tts").join("v3nano"));
        local_search_paths.push(parent.join("models").join("v3nano"));
        local_search_paths.push(parent.join("v3nano"));
    }

    for src_dir in &local_search_paths {
        if src_dir.exists() && REQUIRED_FILES.iter().all(|f| src_dir.join(f).exists()) {
            for file in REQUIRED_FILES {
                let dest = target_dir.join(file);
                if !dest.exists() {
                    fs::copy(src_dir.join(file), &dest)?;
                }
            }
            if REQUIRED_FILES.iter().all(|f| target_dir.join(f).exists()) {
                return Ok(target_dir);
            }
        }
    }

    let api = Api::new().map_err(|e| anyhow!("Không thể khởi tạo HuggingFace API: {}", e))?;
    let repo = api.model(NANO_HF_REPO.to_string());

    for &file in REQUIRED_FILES {
        let dest = target_dir.join(file);
        if !dest.exists() {
            let downloaded_path = repo.get(file).map_err(|e| {
                anyhow!(
                    "Lỗi tải '{}' từ HuggingFace repo '{}': {}",
                    file,
                    NANO_HF_REPO,
                    e
                )
            })?;
            fs::copy(&downloaded_path, &dest)?;
        }
    }

    Ok(target_dir)
}
