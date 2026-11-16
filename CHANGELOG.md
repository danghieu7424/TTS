# Nhật ký thay đổi

Các thay đổi đáng chú ý của workspace được ghi tại đây.

## [Unreleased]

### Thêm
- Thêm workspace với các crates `vieneu-tts-core`, `vieneu-tts-engine` và facade `vieneu-tts`.
- Thêm API cấu hình cho model path, số luồng, DirectML, G2P, FFmpeg và callback sự kiện.
- Thêm parser WebVTT và property tests cho normalizer, DSP, parser.
- Nhúng `voices_v3_turbo.json` vào crate engine.

### Thay đổi
- Cô lập model inference và dubbing pipeline khỏi các API host `shared`/`cli_gui_form`.
- Đưa các module v3-Nano vào feature `nano`; đưa FFmpeg atempo vào feature tùy chọn `ffmpeg`.

### Sửa
- Xử lý an toàn sample rate không hợp lệ và giá trị NaN/Inf trong bộ lọc DSP.
- Báo lỗi rõ ràng khi worker G2P trả thành công nhưng không có phonemes hoặc khi FFmpeg time-stretch thất bại.
- Nâng `ndarray-npy` để dùng `zip` đã vá các advisory hiện hành.

### Gỡ bỏ

### Bảo mật

### Thay đổi phá vỡ tương thích
- API cũ trong `Source/TTS` chưa được giữ làm API tương thích; facade mới dùng `TtsConfig` và `TtsSynthesizer`.
