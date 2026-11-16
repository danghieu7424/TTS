# Trạng thái repository

- Kiểm thử: 12 tests qua với `cargo test --workspace`, `cargo test --workspace --all-features` và `cargo test --workspace --no-default-features`.
- Chất lượng: `cargo fmt --all -- --check` và `cargo clippy --workspace --all-targets -- -D warnings` thành công.
- Audit: không còn vulnerability; `cargo audit` báo 2 crate bắc cầu chưa được duy trì (`number_prefix`, `paste`).
- Ràng buộc: inference ONNX cần runtime tương thích và model files bên ngoài; G2P cần bridge ngoài. Feature Nano giữ các atom cũ, facade hiện dùng Turbo.
- Báo cáo đánh giá thực nghiệm/benchmark: chưa chạy trong phiên này.
