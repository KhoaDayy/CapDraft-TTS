# Changelog

## 1.1.2 — 2026-07-12

### Sửa lỗi

- Chủ đề **Sáng** áp dụng đúng palette sáng cho toàn app (trước đó chỉ đổi control Fluent, nền/table vẫn tối).
- Cân lại layout **Cài đặt giọng đọc**: label căn phải, cột voice rộng hơn, `TTS đã tồn tại` không còn nửa hàng trống.
- Panel nâng cao và toolbar caption khoảng cách gọn hơn.

## 1.1.1 — 2026-07-11

### Build

- Dùng `uv` để tạo `.venv`, cài dependency và chạy toàn bộ test/build.
- Thêm `pyproject.toml` và `uv.lock` để môi trường build có thể tái tạo chính xác.
- Script release tự chạy `uv sync --frozen` và `uv run pyinstaller`, không còn phụ thuộc Python global.

## 1.1.0 — 2026-07-11

### Tính năng

- Giao diện hỗ trợ Tiếng Việt, English, 中文 và 日本語.
- Thêm chủ đề Theo Windows, Sáng và Tối; mặc định theo giao diện hệ thống.
- Đổi màu nhấn sang Windows Blue (`#0078D4`).
- Đưa lựa chọn ngôn ngữ và giao diện vào Settings, không cần sửa `config.json`.

### Dọn dẹp

- Bỏ cấu hình FFmpeg không được sử dụng; chỉ giữ FFprobe tùy chọn để đọc thời lượng audio cache khi thiếu metadata.

## 1.0.2 — 2026-07-11

### Tính năng

- Đọc caption trong project CapCut và xuất trực tiếp thành file SubRip (`.srt`).
- Giữ đúng thứ tự timeline, timestamp mili-giây, caption nhiều dòng và tiếng Việt UTF-8.

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
