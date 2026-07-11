<p align="center">
  <img src="assets/capdraft-tts-banner.png" alt="CapDraft TTS — TTS CapCut miễn phí cho caption" width="100%">
</p>

<h1 align="center">CapDraft TTS</h1>

<p align="center">
  <a href="../README.md">English</a> ·
  <strong>Tiếng Việt</strong> ·
  <a href="README.zh.md">中文</a> ·
  <a href="README.ja.md">日本語</a>
</p>

<p align="center">
  Tạo <strong>120+ giọng TTS CapCut miễn phí</strong> từ caption trong project<br>
  và gắn audio trở lại đúng draft CapCut đó.
</p>

<p align="center">
  <a href="https://github.com/KhoaDayy/CapDraft-TTS/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/KhoaDayy/CapDraft-TTS?label=release"></a>
  <a href="https://github.com/KhoaDayy/CapDraft-TTS/stargazers"><img alt="Stars" src="https://img.shields.io/github/stars/KhoaDayy/CapDraft-TTS?style=social"></a>
  <a href="../LICENSE"><img alt="License" src="https://img.shields.io/github/license/KhoaDayy/CapDraft-TTS"></a>
  <a href="https://www.python.org/"><img alt="Python" src="https://img.shields.io/badge/python-3.10%2B-blue"></a>
  <a href="#yêu-cầu"><img alt="Platform" src="https://img.shields.io/badge/platform-Windows-0078D6"></a>
</p>

<p align="center">
  <img alt="capcut" src="https://img.shields.io/badge/capcut-TTS-black">
  <img alt="text-to-speech" src="https://img.shields.io/badge/text--to--speech-120%2B%20voices-success">
  <img alt="free" src="https://img.shields.io/badge/TTS-free%20%28no%20paid%20API%29-brightgreen">
  <img alt="desktop" src="https://img.shields.io/badge/desktop-PySide6-orange">
  <img alt="video" src="https://img.shields.io/badge/video-editing-lightgrey">
</p>

---

## Vì sao có tool này

CapCut đã có sẵn nhiều giọng TTS. CapDraft TTS giúp bạn dùng chúng trên caption **mà không cần dịch vụ TTS trả phí bên thứ ba**.

- **120+ giọng CapCut** (catalog online từ GitHub)
- **Không mất phí TTS** — đi đường TTS của CapCut qua API local + `device.json`
- Đọc caption từ project CapCut và ghi audio lại đúng timeline đó
- Không copy project, không kéo thả audio thủ công

> Vẫn cần CapCut Desktop, bộ CapCut TTS API hoạt động được, và internet. “Miễn phí” nghĩa là không trả thêm API giọng (ElevenLabs, Azure, …).

## Tính năng

- Mở thư mục project CapCut hoặc file `draft_content.json`
- Danh sách caption: tìm kiếm, chọn tất cả, lọc rỗng / chưa TTS / lỗi
- Catalog giọng online (~120+) kèm lọc ngôn ngữ + tìm kiếm
- Tốc độ TTS, tốc độ clip CapCut, chế độ cao độ
- Thay TTS cũ hoặc bỏ qua caption đã có TTS
- Cache audio (tuỳ chọn) và căn đầu audio native CapCut
- Sinh song song, huỷ tác vụ, log tiến trình
- Backup + ghi atomic, rollback khi lỗi
- UI Fluent dark mode (giao diện tiếng Việt)

## Yêu cầu

- Windows 10/11
- CapCut Desktop + project đã có caption
- CapCut TTS API local kèm `device.json` hợp lệ
- FFmpeg / FFprobe (trong `PATH` hoặc cấu hình trong Settings)
- Internet (tải catalog giọng + gọi TTS CapCut)

Chạy từ source cần thêm Python 3.10+.

## Cài đặt

### Bản dựng sẵn (khuyên dùng)

1. Tải `CapDraft-TTS-v1.0.1-windows-x64.zip` từ [Releases](https://github.com/KhoaDayy/CapDraft-TTS/releases)
2. Giải nén và chạy `CapDraft-TTS.exe`
3. Mở **Cài đặt** (bánh răng) rồi chỉ:
   - Thư mục CapCut TTS API
   - `device.json`
   - FFmpeg / FFprobe nếu chưa có trong `PATH`

```powershell
Get-FileHash .\CapDraft-TTS-v1.0.1-windows-x64.zip -Algorithm SHA256
# so với file .sha256 trong release
```

### Chạy từ source

```powershell
git clone https://github.com/KhoaDayy/CapDraft-TTS.git
cd CapDraft-TTS
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install -r requirements.txt
Copy-Item config.example.json config.json
python main.py
```

## Cách dùng

1. **Chọn project** → thư mục project CapCut hoặc `draft_content.json`
2. Xem / lọc caption
3. Chọn ngôn ngữ + giọng (120+ giọng CapCut từ catalog online)
4. Chỉnh tốc độ, clip speed, cao độ, chính sách TTS cũ
5. Chọn caption → **Tạo và gắn TTS**
6. Đóng project trong CapCut trước khi ghi, rồi mở lại CapCut để kiểm tra timeline

> [!IMPORTANT]
> Hãy đóng project CapCut trước khi ghi. App có backup local, nhưng vẫn nên tự sao lưu draft quan trọng.

## TTS CapCut miễn phí hoạt động thế nào

```text
Caption trong project CapCut
        │
        ▼
 CapDraft TTS  ──►  CapCut TTS API local + device.json  ──►  giọng TTS CapCut
        │
        ▼
 audio gắn lại vào đúng draft timeline
```

- Giọng lấy từ catalog CapCut (liệt kê online qua `Voice.json` trên repo này)
- Sinh giọng qua bộ CapCut TTS API của bạn, **không** qua cloud TTS trả phí
- App không bán giọng, không tính phí theo ký tự

## Catalog giọng

Load **trực tiếp vào RAM** từ:

```text
https://raw.githubusercontent.com/KhoaDayy/CapDraft-TTS/refs/heads/main/Voice.json
```

- Package app không cần file catalog local
- Đổi URL hoặc bấm **Tải lại danh sách** trong **Cài đặt → Giọng đọc**
- Muốn thêm/sửa giọng cho mọi người: cập nhật `Voice.json` trên nhánh `main`

## Cấu hình

`config.json` theo máy (git ignore). Copy từ [`config.example.json`](../config.example.json).

| Key | Mục đích |
| --- | --- |
| `capcut_tts_path` | Thư mục CapCut TTS API |
| `device_json_path` | `device.json` cho CapCut TTS |
| `voice_catalog_url` | URL raw danh sách giọng |
| `ffmpeg_path` / `ffprobe_path` | Công cụ media |
| `tts_chunk_size` | Số caption mỗi batch |
| `tts_parallel_chunks` | Số batch song song |
| `tts_download_workers` | Số luồng tải audio |
| `tts_poll_interval_sec` | Chu kỳ poll khi chờ TTS |
| `cache_path` | Cache audio đã sinh |
| `max_backups` | Số backup draft giữ lại |

## Cấu trúc project

```text
main.py                 Điểm vào app
core/config.py          Cấu hình
core/capcut_tts.py      Wrapper CapCut TTS API
core/capcut_project/    Đọc / patch / validate draft CapCut
ui/                     Giao diện desktop
tests/                  Test
Voice.json              Danh sách giọng public (fetch bằng URL)
docs/                   README đa ngôn ngữ
```

## Phát triển

```powershell
python -m unittest discover -s tests -v
.\build-release.ps1 -Version 1.0.1
```

Xem [`CONTRIBUTING.md`](../CONTRIBUTING.md) và [`CHANGELOG.md`](../CHANGELOG.md).

## Giấy phép / pháp lý

- [`LICENSE`](../LICENSE) (Apache-2.0)
- Dự án độc lập — **không** liên kết hay được CapCut / ByteDance bảo trợ
- Bạn tự chịu trách nhiệm về điều khoản CapCut, tài khoản/thiết bị, và bản quyền nội dung
- QFluentWidgets GPLv3 cho mục đích phi thương mại; kiểm tra license trước khi phân phối thương mại
