# Điều tra nhiễu âm thanh TTS — 2026-11-16

## Kết luận tóm tắt

- VTT `S02E001.merged.zh.vi.vtt` có 58 cue; parser loại 6 cue `line:20%` và giữ 52 cue thoại. Bộ tổng hợp không đưa các caption này vào hàng đợi đọc.
- Codec config khai báo `audio_sample_rate: 48000`. Engine giải mã tensor stereo `[1, 2, n_samples]`, lấy trung bình thành mono; không có bước resample hoặc upsample trong đường giải mã.
- Trước sửa, hai rủi ro DSP được xác nhận trong mã: hệ số `alpha` của biquad thấp hơn công thức Butterworth Q=1/√2 một nửa và WSOLA không bypass sai số nhỏ/không giới hạn tốc độ nén. Không tìm thấy clipping PCM16 trong các file chẩn đoán.
- Đã sửa hệ số biquad, WSOLA và giới hạn PCM; test sin ở 1.0x/1.15x, đáp tuyến -3 dB và biên PCM đều đạt.

## Mẫu chẩn đoán

Các WAV được xuất ở `output/debug_noise/` (PCM16 mono, 48 kHz). Đây là các file đầu ra chẩn đoán được ghi tường minh; chúng không phải file tạm của pipeline.

| File | Cue | Thời lượng | Peak | Mẫu clipping |
|---|---:|---:|---:|---:|
| `01_raw_engine.wav` | 25 | 2.640 s | 0.559 | 0 |
| `02_after_dsp.wav` | 25 | 2.450 s | 0.616 | 0 |
| `03_after_wsola.wav` | 25 | 1.815 s | 0.614 | 0 |
| `short_01_raw_engine.wav` | 1 | 0.720 s | 0.525 | 0 |
| `short_02_after_dsp.wav` | 1 | 0.480 s | 0.476 | 0 |
| `short_03_after_wsola.wav` | 1 | 0.480 s | 0.476 | 0 |
| `voice_anh_khoi.wav` | 1, voice Anh Khôi | 0.430 s | 0.631 | 0 |
| `voice_mai_anh.wav` | 1, voice Mai Anh | 0.510 s | 0.424 | 0 |

Cue 1 (“Đến đây.”) có 7.04 s khả dụng và không cần WSOLA. Cue 25 (“Ta quyết định sẽ trở thành thú hợp đồng của ngài một lần nữa.”) có 1.64 s khả dụng; sau trim/DSP còn 2.45 s, yêu cầu nén 1.49x. Ngưỡng mới giới hạn tỷ lệ ở 1.35x nên stage WSOLA dài 1.815 s. Pipeline phát cảnh báo và dời các cue kế tiếp thay vì nén thêm tới mức rủi ro méo tiếng.

Hai WAV voice phụ dùng cùng câu ngắn và cùng phonemes với `Trúc Ly`, để tách biệt ảnh hưởng của voice embedding. Các phép đo peak/PCM không thể thay thế đánh giá nghe chủ quan; nếu còn nghe thấy nhiễu, cần xác định thời điểm và đối chiếu trực tiếp ba stage.

## Thay đổi DSP

- WSOLA dùng cửa sổ 40 ms (1,920 mẫu ở 48 kHz), overlap 50% (960 mẫu), search ±5 ms (240 mẫu) với bước tìm 8 mẫu. Crossfade cosine kiểu Hann có hai trọng số bổ sung, giúp tổng trọng số tại vùng chồng lấp bằng 1; lựa chọn điểm ghép dùng normalized cross-correlation.
- Bypass khi `|speed_ratio - 1| < 0.03`; giới hạn nén tối đa 1.35x.
- Biquad dùng `alpha = sin(omega)/(2Q)` với `Q = 1/√2`; test tại 48 kHz xác nhận gain ở cutoff gần -3 dB cho cả high-pass và low-pass.
- `f32_to_i16_pcm` xử lý non-finite, soft-limit từ 0.98 và clamp trước khi đổi sang i16. Test xác nhận không wrap-around.

## Release run và dữ liệu nguồn

Lệnh chạy: `cargo run --release -p vieneu-tts --example evaluate_vtt -- --out-dir output .\input\in\S02E001.merged.zh.vi.vtt`.

| Cue tổng | Caption lọc | Cue đọc | Thời gian xử lý | Thời lượng WAV | RTF |
|---:|---:|---:|---:|---:|---:|
| 58 | 6 | 52 | 32.436 s | 141.350 s | 0.2295 |

WAV `output/S02E001.wav` là mono PCM16, 48 kHz; peak 0.8363, không có mẫu clipping. SHA-256 VTT trước và sau chạy đều là `92EE8C62EA1A0B5655A3A9B1FD3D53BC830FA13A47C00384C978C655E5295ADD`.

## Kiểm tra

- `cargo fmt --all -- --check`: đạt.
- `cargo test --workspace --all-features`: đạt, 21 test.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: đạt.
- `cargo audit`: không có lỗ hổng; còn hai cảnh báo dependency bắc cầu chưa được duy trì (`number_prefix`, `paste`), không có bản sửa được audit đề xuất.
