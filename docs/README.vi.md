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

---

## Vì sao có tool này

CapCut đã có nhiều giọng TTS. CapDraft TTS giúp dùng chúng trên caption **không cần API TTS trả phí bên thứ ba**.

- **120+ giọng CapCut**
- Đọc caption từ project và ghi audio lại đúng timeline
- Project lớn (~5k caption): bảng ảo hóa, draft gọn hơn để export
- **Tách 2 phần** project CapCut → export từng nửa → **Ghép video** bằng ffmpeg
- Xuất caption ra `.srt`

> Vẫn cần CapCut Desktop và internet. “Miễn phí” = không mua thêm dịch vụ TTS riêng.

## Tính năng

| Hạng mục | Chi tiết |
|----------|----------|
| **Project** | Mở thư mục project hoặc `draft_content.json` (kể cả layout `Timelines/`) |
| **Caption** | Bảng ảo hóa ~5k dòng; tìm; chọn hết; lọc rỗng / chưa TTS / lỗi |
| **Giọng** | Catalog online ~120+, lọc ngôn ngữ + tìm |
| **TTS** | Tốc độ clip, cao độ, thay/bỏ qua TTS cũ, cache, căn đầu native |
| **Pipeline** | Sinh song song, huỷ, log, backup + ghi atomic + rollback |
| **Export** | **Tách 2 phần** · **Ghép video** (ffmpeg) · **Xuất SRT** |
| **UI** | VI / EN / ZH / JA · theme Windows sáng/tối/auto |

## Yêu cầu

- Windows 10/11
- CapCut Desktop + project đã có caption
- Internet (TTS + catalog giọng)
- **ffmpeg** trong `PATH` (hoặc `ffmpeg_path`) — chỉ cần cho **Ghép video**
- **ffprobe** tuỳ chọn — đọc duration cache

Phát triển từ source cần [`uv`](https://docs.astral.sh/uv/).

## Cài đặt

### Bản dựng sẵn (khuyên dùng)

1. Tải `CapDraft-TTS-v*-windows-x64.zip` mới nhất từ [Releases](https://github.com/KhoaDayy/CapDraft-TTS/releases/latest)
2. Giải nén, chạy `CapDraft-TTS.exe`
3. Chọn project CapCut; thường không cần sửa `config.json`

```powershell
Get-FileHash .\CapDraft-TTS-v1.2.4-windows-x64.zip -Algorithm SHA256
# so với file .sha256 trong release
```

### Từ source

```powershell
git clone https://github.com/KhoaDayy/CapDraft-TTS.git
cd CapDraft-TTS
uv sync --group dev
Copy-Item config.example.json config.json
uv run python main.py
```

Tuỳ chọn trong `config.json` (xem `config.example.json`): `capcut_projects_path`, `ffmpeg_path`, `tts_chunk_size`, `language`, `theme_mode`, …

## Cách dùng

### Tạo TTS

1. **Chọn project** → thư mục CapCut hoặc `draft_content.json`
2. Xem / lọc caption
3. Chọn ngôn ngữ + giọng
4. Chỉnh tốc độ clip, cao độ, TTS đã tồn tại
5. Chọn caption → **Tạo và gắn TTS**
6. **Đóng project CapCut** trước khi ghi, rồi mở lại để kiểm tra

### Project lớn / export CapCut fail

1. Load project (sau khi gắn TTS cũng được)
2. **Tách 2 phần** → tạo `<tên>_part1` và `_part2` cạnh project gốc
3. Thoát hẳn CapCut, mở lại, export từng part
4. **Ghép video** → chọn video part1 rồi part2 → lưu 1 file MP4

> [!IMPORTANT]
> Đóng CapCut (hoặc project) trước khi ghi TTS / tách project. App có backup khi ghi TTS, nhưng vẫn nên tự sao lưu draft quan trọng.

### Xuất SRT

**Xuất SRT** lưu caption không rỗng thành file `.srt`.

## Phát triển

```powershell
uv run python -m unittest discover -s tests -v
.\build-release.ps1 -Version 1.2.4
```

Release: push tag `v*` (CI build Windows + GitHub Release). Xem [`CHANGELOG.md`](../CHANGELOG.md), [`CONTRIBUTING.md`](../CONTRIBUTING.md).

## Giấy phép / pháp lý

- [`LICENSE`](../LICENSE) (Apache-2.0)
- Dự án độc lập — **không** liên kết CapCut / ByteDance
- Bạn tự chịu trách nhiệm điều khoản CapCut, tài khoản/thiết bị, bản quyền nội dung
- QFluentWidgets GPLv3 phi thương mại; kiểm tra license trước khi phân phối thương mại
