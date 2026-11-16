# VieNeu TTS Rust Workspace

Thư viện Rust có thể nhúng để chuẩn hóa tiếng Việt, xử lý DSP và tổng hợp/lồng tiếng bằng VieNeu-TTS v3-Turbo. Mã mẫu ban đầu được tách thành ba crates: `vieneu-tts-core`, `vieneu-tts-engine` và facade `vieneu-tts`.

## Môi trường

- Rust stable với edition 2024 và Cargo.
- Windows: bộ công cụ MSVC C++/Windows SDK nếu target Rust của máy yêu cầu MSVC.
- Runtime ONNX tương thích với phiên bản `ort` của crate, có thể nạp động; thêm DLL ONNX Runtime vào `PATH` khi chạy inference. DirectML chỉ được chọn khi cấu hình `use_gpu` bật trên Windows.
- FFmpeg chỉ cần khi bật feature `ffmpeg` (được bật mặc định) và cue cần time-stretch. Có thể truyền đường dẫn cụ thể qua `TtsConfig::ffmpeg_path`.
- G2P cần binary bridge đã build hoặc Python cùng bridge script. Có thể cấu hình qua `g2p_bridge_path`/`python_path`; bridge script đặt trong thư mục model cũng được dò tìm.
- Thư mục model truyền vào `TtsConfig::new` có thể chứa `v3turbo/` để dùng asset cục bộ; các file còn thiếu được tải từ Hugging Face. Voice presets được đóng gói cùng crate.

## Dùng thư viện

```rust,no_run
use std::path::Path;
use vieneu_tts::{TtsConfig, TtsSynthesizer};

let config = TtsConfig::new("Models")
    .threads(4)
    .use_gpu(false);
let synthesizer = TtsSynthesizer::new(config);
let result = synthesizer.synthesize_dubbed_audio(
    Path::new("input.vi.vtt"),
    Path::new("work"),
    "Trúc Ly",
    false,
)?;
# Ok::<(), anyhow::Error>(())
```

`model_dir` là thư mục cha chứa `v3turbo/` hoặc thư mục cache `tts/v3turbo/`. Facade không cần các module nội bộ của ứng dụng host; tiến trình/log được gửi qua callback `TtsConfig::on_event`.

## Crates và features

| Crate | Trách nhiệm |
|---|---|
| `vieneu-tts-core` | Vietnamese normalizer, DSP, WebVTT parser và event types |
| `vieneu-tts-engine` | VieNeu-TTS ONNX engine, model downloader và G2P IPC |
| `vieneu-tts` | API facade và dubbing timeline |

- `turbo`: engine VieNeu-TTS v3-Turbo; mặc định bật trong facade.
- `nano`: biên dịch các module v3-Nano cũ dưới feature tùy chọn.
- `ffmpeg`: bật time-stretch qua FFmpeg; mặc định bật trong facade. Tắt feature thì các cue bị dài hơn giới hạn thời lượng không được time-stretch.

## Giới hạn hiện tại

- Inference cần model ONNX và ONNX Runtime phù hợp tại runtime; không đóng gói các weights lớn trong crate.
- G2P hiện vẫn là tiến trình bridge ngoài, không phải implementation Rust thuần.
- Feature `nano` chỉ giữ các atom Nano mẫu; facade dubbing hiện sử dụng engine Turbo.
- `silence_gaps.json`, `vocals.wav`, phân tích pitch/gender, loudness normalization và mux với video chưa được triển khai.
- Khi tắt `ffmpeg`, timeline vẫn căn cue theo start time nhưng không bảo đảm co giãn audio dài về target duration.

## Kiểm tra

```text
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo audit
```
