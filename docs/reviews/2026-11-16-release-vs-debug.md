# WAV nghe thử: release so với debug — 2026-11-16

## Môi trường và phương pháp

- Windows, 12 logical processors; 4 worker, 3 intra-thread/worker; cùng model Turbo, G2P bridge, voice `Trúc Ly`, WSOLA và bốn file đầu vào như [lượt debug](./2026-11-16-evaluate.md).
- ONNX Runtime 1.22.0 tải từ release chính thức `microsoft/onnxruntime` và được giữ lâu dài tại `Models/onnxruntime.dll`; `Models/onnxruntime_providers_shared.dll` cũng được đặt cạnh DLL để thỏa dependency runtime. Cả `Models/` và `output/` đều bị ignore bởi Git.
- Command: `cargo run --release -p vieneu-tts --example evaluate_vtt -- --out-dir output`, với `ORT_DYLIB_PATH` trỏ tới `Models/onnxruntime.dll` và `Models/` được thêm vào `PATH`.
- Harness đo từng `synthesize_dubbed_audio` bằng `Instant`; RTF = thời gian tổng hợp / độ dài timeline output. Tổng batch bao gồm mã hóa/ghi bốn WAV. Các đầu vào chỉ được đọc, không bị sửa.

## Kết quả release

| Input | Output | Cue / lọc / đọc | Tổng hợp (s) | Output (s) | RTF | WAV bytes |
|---|---|---:|---:|---:|---:|---:|
| `S02E001.merged.zh.vi.vtt` | `S02E001.wav` | 58 / 6 / 52 | 33.386 | 141.350 | 0.2362 | 13,569,644 |
| `S02E002.merged.zh.vi.vtt` | `S02E002.wav` | 82 / 19 / 63 | 52.298 | 142.050 | 0.3682 | 13,636,844 |
| `第001集.merged.zh.vi.vtt` | `第001集.merged.wav` | 71 / 7 / 64 | 42.842 | 146.650 | 0.2921 | 14,078,444 |
| `第001集.zh.vi.vtt` | `第001集.wav` | 61 / 0 / 61 | 42.287 | 146.650 | 0.2884 | 14,078,444 |
| **Tổng** | **4 WAV** | **272 / 32 / 240** | **170.813** | **576.700** | **0.2962** | **55,363,376** |

Tổng `cargo run` hoàn tất trong **172.895 s** (1.7 s incremental build; 170.864 s batch, gồm ghi WAV). Bản release đạt khoảng **3.38× thời gian phát** và nhanh hơn lượt debug **11.61×** theo tổng thời gian synthesis. RTF debug là 3.4381 từ báo cáo trước. RTF không tính WAV write; phần này thêm khoảng 0.051 s cho cả batch.

Cả bốn WAV đã được kiểm tra bằng parser RIFF: mono, PCM16, 48,000 Hz; thời lượng đúng với bảng. SHA-256 của bốn VTT sau lượt chạy vẫn trùng baseline trong báo cáo debug. File nghe thử nằm tại `output/` và không được track vào Git.

## Kiểm toán G2P trước khi sửa engine

`Models/g2p_bridge/_internal/` chứa runtime Python 3.12, NumPy và thư viện native; các tài nguyên đáng chú ý là `sea_g2p/sea_g2p.bin` (62,829,820 bytes), `sea_g2p/sea_g2p_rs.pyd` (5,723,136 bytes) và `base_library.zip` (155 mục thư viện chuẩn). Không thấy `.py`, `.pyc`, `.pyo`, wheel, metadata hay license cho G2P trong thư mục này; license nhìn thấy được là của NumPy. Không thấy module `vig2p`/`vieneu_utils` trong `base_library.zip`.

Bridge giao tiếp bằng JSON Lines: request `{"text":"..."}` và response `{"success":true,"phonemes":"..."}` (hoặc `error`). Smoke test “Xin chào thế giới.” trả một chuỗi Unicode có khoảng trắng và ký hiệu IPA, ví dụ `sˈin sˌiːˈeɪtʃ? ˈoʊ tˌiːˈeɪtʃ? z? ˈi.`; đây chỉ xác nhận giao thức/định dạng trả về, không xác nhận chất lượng ánh xạ cho mọi tiếng Việt. `VietNEUTurboEngine` nhận chuỗi này trực tiếp và gọi `tokenizer.encode(phonemes, false)`.

Các file hiển thị không chứa bảng âm vị hay từ điển có thể đọc/đối chiếu. Phần thực thi và tài nguyên G2P còn lại là executable/native binary không có license kèm theo; chưa trích xuất bytecode, decompile hoặc sao chép logic/từ điển. Cần xác nhận quyền sử dụng/biên dịch lại hoặc cung cấp source có license trước khi tiếp tục port tương đương sang Rust.
