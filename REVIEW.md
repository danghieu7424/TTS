# Trạng thái repository

- Phiên bản workspace: `0.3.0`; `Source/`, root `src/` và `plan.md` đã được dọn theo xác nhận.
- Kiểm thử: 17 tests qua với cấu hình mặc định và `--all-features`; 15 tests qua với `--no-default-features`.
- Chất lượng: `cargo fmt --all -- --check` và `cargo clippy --workspace --all-targets --all-features -- -D warnings` thành công.
- Audit: không có vulnerability; `cargo audit` còn 2 cảnh báo dependency bắc cầu chưa được duy trì (`number_prefix`, `paste`), chưa có bản sửa.
- Ràng buộc: inference yêu cầu ONNX Runtime 1.22.x tương thích; pipeline tạo audio trong RAM nhưng model cache vẫn lưu ở `model_dir`. G2P còn là bridge ngoài; source/license không có trong bundle để port an toàn.
- Đánh giá debug: [2026-11-16-evaluate.md](docs/reviews/2026-11-16-evaluate.md) — 272 cue, 32 caption bị lọc, RTF tổng 3.4381, peak process-tree working set 2,197.2 MiB; hash input không đổi.
- Đánh giá release: [2026-11-16-release-vs-debug.md](docs/reviews/2026-11-16-release-vs-debug.md) — 4 WAV nghe thử được lưu ở `output/`, RTF tổng 0.2962 (nhanh hơn debug 11.61 lần).
