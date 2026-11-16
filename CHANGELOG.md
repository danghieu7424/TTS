# Nhật ký thay đổi

Các thay đổi đáng chú ý của workspace được ghi tại đây.

## [Unreleased]

### Thêm
- Thêm workspace với các crates `vieneu-tts-core`, `vieneu-tts-engine` và facade `vieneu-tts`.
- Thêm API cấu hình cho model path, số worker/luồng, DirectML, G2P và callback sự kiện.
- Thêm parser WebVTT và property tests cho normalizer, DSP, parser.
- Nhúng `voices_v3_turbo.json` vào crate engine.
- Thêm `TtsConfig::num_workers`, `AudioBuffer`, WAV encoding trong RAM, thống kê caption lọc và WSOLA.
- Thêm tùy chọn `--out-dir` cho ví dụ `evaluate_vtt` để lưu WAV nghe thử.
- Thêm ví dụ `debug_noise` xuất WAV từng tầng engine/DSP/WSOLA và kiểm tra hai voice preset.

### Thay đổi
- Cô lập model inference và dubbing pipeline khỏi các API host `shared`/`cli_gui_form`.
- `parse_vtt_with_stats` lọc cue `line:<n>%` dưới 50%; pipeline loại chữ Hán trước G2P.
- Dùng hàng đợi cue động với tối đa bốn worker ONNX/G2P, gom kết quả theo thứ tự cue.
- Đưa các module v3-Nano vào feature `nano`; time-stretch chuyển sang WSOLA trong RAM.

### Sửa
- Xử lý an toàn sample rate không hợp lệ và giá trị NaN/Inf trong bộ lọc DSP.
- Sửa hệ số Q Butterworth, bỏ qua WSOLA khi chênh lệch dưới 3%, dùng crossfade Hann bổ sung và giới hạn nén tối đa 1.35x.
- Thêm soft limiter PCM16 từ ngưỡng 0.98 và kiểm thử sin, đáp tuyến biquad, clipping.
- Báo lỗi rõ ràng khi worker G2P trả thành công nhưng không có phonemes hoặc khi FFmpeg time-stretch thất bại.
- Nâng `ndarray-npy` để dùng `zip` đã vá các advisory hiện hành.

### Gỡ bỏ
- Bỏ FFmpeg và các file WAV trung gian khỏi pipeline dubbing.

### Bảo mật

### Thay đổi phá vỡ tương thích
- API cũ trong `Source/TTS` chưa được giữ làm API tương thích; facade mới dùng `TtsConfig` và `TtsSynthesizer`.
- `TtsSynthesizer::synthesize_dubbed_audio` không còn nhận `tmp_dir`/`force_tts`; kết quả audio được trả trong `TTSOutput::audio`.
- Bỏ `TtsConfig::ffmpeg_path`; `threads` là ngân sách tổng được chia giữa các worker.
