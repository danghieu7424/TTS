# VieNeu TTS Rust Workspace

Thư viện Rust có thể nhúng để chuẩn hóa tiếng Việt, xử lý DSP và tổng hợp/lồng tiếng bằng VieNeu-TTS v3-Turbo. Mã mẫu ban đầu được tách thành ba crates: `vieneu-tts-core`, `vieneu-tts-engine` và facade `vieneu-tts`.

## Môi trường

- Rust stable với edition 2024 và Cargo.
- Windows: bộ công cụ MSVC C++/Windows SDK nếu target Rust của máy yêu cầu MSVC.
- Runtime ONNX tương thích với phiên bản `ort` của crate, có thể nạp động; thêm DLL ONNX Runtime vào `PATH` khi chạy inference. DirectML chỉ được chọn khi cấu hình `use_gpu` bật trên Windows.
- G2P cần binary bridge đã build hoặc Python cùng bridge script. Cấu hình qua `g2p_bridge_path`/`python_path`; bridge đặt trong thư mục model cũng được dò tìm.
- Thư mục model truyền vào `TtsConfig::new` có thể chứa `v3turbo/` để dùng asset cục bộ; các file còn thiếu được tải từ Hugging Face và lưu trong `model_dir/tts/v3turbo`. Voice presets được đóng gói cùng crate.
- Windows local evaluation có thể dùng ONNX Runtime 1.22.x tại `Models/onnxruntime.dll` bằng cách đặt `ORT_DYLIB_PATH` trỏ tới DLL và thêm `Models/` vào `PATH`.

## Dùng thư viện

```rust,no_run
use std::path::Path;
use vieneu_tts::{TtsConfig, TtsSynthesizer};

let config = TtsConfig::new("Models")
    .num_workers(4)
    .threads(8)
    .use_gpu(false);
let synthesizer = TtsSynthesizer::new(config);
let result = synthesizer.synthesize_dubbed_audio(
    Path::new("input.vi.vtt"),
    "Trúc Ly",
)?;
let wav_bytes = result.audio.wav_bytes()?;
std::fs::write("dubbed.wav", wav_bytes)?;
# Ok::<(), anyhow::Error>(())
```

`model_dir` là thư mục cha chứa `v3turbo/` hoặc thư mục cache `tts/v3turbo/`. Facade không cần các module nội bộ của ứng dụng host; tiến trình/log được gửi qua callback `TtsConfig::on_event`.
`TTSOutput` trả audio mono PCM16/48 kHz trong `AudioBuffer`, cùng tổng cue, caption bị lọc, cue đã đọc và voice. `AudioBuffer::wav_bytes()` mã hóa WAV trong RAM; chỉ caller quyết định có ghi bytes ra file hay không.

Pipeline dùng hàng đợi động với tối đa bốn worker theo mặc định. Mỗi worker giữ engine ONNX và G2P bridge riêng; `threads` là ngân sách tổng luồng nội suy, được chia cho worker. Cue `line:<n>%` dưới 50% bị loại; cue lỗi cú pháp `line:` vẫn được giữ và phát cảnh báo. Ký tự chữ Hán bị loại trước G2P, phần tiếng Việt và Latin còn lại được đọc.

Để tạo WAV nghe thử từ bốn VTT mẫu trên Windows:

```powershell
$env:ORT_DYLIB_PATH = (Resolve-Path "Models\onnxruntime.dll").Path
$env:PATH = (Resolve-Path "Models").Path + ";" + $env:PATH
cargo run --release -p vieneu-tts --example evaluate_vtt -- --out-dir output
```

Harness ghi WAV mono PCM16/48 kHz và in thời gian/RTF cho mỗi file. `output/` và `Models/` nằm ngoài Git theo `.gitignore`.

## Crates và features

| Crate | Trách nhiệm |
|---|---|
| `vieneu-tts-core` | Vietnamese normalizer, DSP, WebVTT parser và event types |
| `vieneu-tts-engine` | VieNeu-TTS ONNX engine, model downloader và G2P IPC |
| `vieneu-tts` | API facade và dubbing timeline |

- `turbo`: engine VieNeu-TTS v3-Turbo; mặc định bật trong facade.
- `nano`: biên dịch các module v3-Nano cũ dưới feature tùy chọn.
- Time-stretch cue được thực hiện bằng WSOLA thuần Rust trong RAM, không cần feature hay subprocess FFmpeg.

## Giới hạn hiện tại

- Inference cần model ONNX và ONNX Runtime phù hợp tại runtime; không đóng gói các weights lớn trong crate.
- G2P hiện vẫn là tiến trình bridge ngoài, không phải implementation Rust thuần.
- Mỗi worker nạp một bộ ONNX sessions riêng; tăng số worker làm tăng RAM đáng kể. Model cache được lưu vào `model_dir`; chỉ audio trung gian không ghi ra đĩa.
- Feature `nano` chỉ giữ các atom Nano mẫu; facade dubbing hiện sử dụng engine Turbo.
- `silence_gaps.json`, `vocals.wav`, phân tích pitch/gender, loudness normalization và mux với video chưa được triển khai.

## Kiểm tra

```text
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo audit
```
