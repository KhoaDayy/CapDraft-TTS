# Changelog

## 1.0.1 — 2026-07-11

### Tính năng

- Tải danh sách giọng trực tiếp từ GitHub URL (mặc định `refs/heads/main/Voice.json`); không cần file `Voice.json` local trong package.
- Settings → **Giọng đọc**: cấu hình URL + nút **Tải lại danh sách**.
- Sửa lại chữ tiếng Việt trong Settings cho sạch hơn.

### Thay đổi

- Bỏ phụ thuộc `voice_catalog_path` / file local; config dùng `voice_catalog_url`.
- Package Windows không còn bundle `Voice.json` — app luôn fetch online (cùng yêu cầu mạng với CapCut TTS API).

### Phân phối

- Windows portable x64; giải nén ZIP và chạy `CapDraft-TTS.exe`.
- Kèm checksum SHA-256 để kiểm tra file tải xuống.

## 1.0.0 — 2026-07-11

Bản phát hành Windows đầu tiên của CapDraft TTS.

### Tính năng

- Đọc caption và gắn TTS trực tiếp vào project CapCut.
- 129 giọng đọc, tìm kiếm/lọc caption và điều khiển tốc độ/cao độ.
- Cache, xử lý song song, hủy tác vụ và log tiến trình.
- Native audio alignment, validation, backup, atomic save và rollback.
- Giao diện Fluent dark mode cùng trang cấu hình đường dẫn/hiệu năng.

### Phân phối

- Windows portable x64; giải nén ZIP và chạy `CapDraft-TTS.exe`.
- Kèm checksum SHA-256 để kiểm tra file tải xuống.
