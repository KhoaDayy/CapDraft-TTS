<p align="center">
  <img src="docs/assets/capdraft-tts-banner.png" alt="CapDraft TTS — free CapCut TTS for your captions" width="100%">
</p>

<h1 align="center">CapDraft TTS</h1>

<p align="center">
  <strong>English</strong> ·
  <a href="docs/README.vi.md">Tiếng Việt</a> ·
  <a href="docs/README.zh.md">中文</a> ·
  <a href="docs/README.ja.md">日本語</a>
</p>

<p align="center">
  Generate <strong>120+ CapCut TTS voices for free</strong> from your project captions<br>
  and attach the audio back into the same CapCut draft.
</p>

<p align="center">
  <a href="https://github.com/KhoaDayy/CapDraft-TTS/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/KhoaDayy/CapDraft-TTS?label=release"></a>
  <a href="https://github.com/KhoaDayy/CapDraft-TTS/stargazers"><img alt="Stars" src="https://img.shields.io/github/stars/KhoaDayy/CapDraft-TTS?style=social"></a>
  <a href="LICENSE"><img alt="License" src="https://img.shields.io/github/license/KhoaDayy/CapDraft-TTS"></a>
  <a href="https://www.python.org/"><img alt="Python" src="https://img.shields.io/badge/python-3.10%2B-blue"></a>
  <a href="#requirements"><img alt="Platform" src="https://img.shields.io/badge/platform-Windows-0078D6"></a>
</p>

<p align="center">
  <img alt="capcut" src="https://img.shields.io/badge/capcut-TTS-black">
  <img alt="text-to-speech" src="https://img.shields.io/badge/text--to--speech-120%2B%20voices-success">
  <img alt="free" src="https://img.shields.io/badge/TTS-free%20%28no%20paid%20API%29-brightgreen">
  <img alt="desktop" src="https://img.shields.io/badge/desktop-PySide6-orange">
  <img alt="video" src="https://img.shields.io/badge/video-editing-lightgrey">
</p>

---

## Why this exists

CapCut already has many TTS voices. CapDraft TTS lets you use them on your captions **without paid third-party TTS services**.

- **120+ CapCut voices** (live catalog from GitHub)
- **No TTS fee** — uses CapCut's own TTS path via a local CapCut TTS API + your `device.json`
- Reads captions from your CapCut project and writes audio back into the same timeline
- No project copy, no manual drag-and-drop of audio files

> You still need CapCut Desktop, a working CapCut TTS API setup, and internet. “Free” means no extra paid voice API (ElevenLabs, Azure, etc.).

## Features

- Open a CapCut project folder or `draft_content.json`
- Caption list with search, select-all, empty / no-TTS / error filters
- Online voice catalog (~120+ voices) with language filter + search
- TTS rate, CapCut clip speed, pitch mode (follow speed / preserve pitch)
- Replace existing TTS or skip captions that already have TTS
- Optional audio cache and native CapCut head-alignment
- Parallel generation, cancel, progress log
- Backup + atomic save with rollback on failed write
- Fluent dark-mode UI (Vietnamese UI text)

## Requirements

- Windows 10/11
- CapCut Desktop + a project that already has captions
- Local CapCut TTS API install with a valid `device.json`
- FFmpeg / FFprobe (`ffmpeg` / `ffprobe` on `PATH`, or set paths in Settings)
- Internet (voice catalog + CapCut TTS requests)

Source runs also need Python 3.10+.

## Install

### Prebuilt (recommended)

1. Download `CapDraft-TTS-v1.0.1-windows-x64.zip` from [Releases](https://github.com/KhoaDayy/CapDraft-TTS/releases)
2. Unzip and run `CapDraft-TTS.exe`
3. Open **Settings** (gear) and set:
   - CapCut TTS API folder
   - `device.json`
   - FFmpeg / FFprobe if not on `PATH`

```powershell
Get-FileHash .\CapDraft-TTS-v1.0.1-windows-x64.zip -Algorithm SHA256
# compare with the .sha256 file from the release
```

### From source

```powershell
git clone https://github.com/KhoaDayy/CapDraft-TTS.git
cd CapDraft-TTS
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install -r requirements.txt
Copy-Item config.example.json config.json
python main.py
```

## Usage

1. **Chọn project** → CapCut project folder or `draft_content.json`
2. Review / filter captions
3. Pick language + voice (120+ CapCut voices from the online catalog)
4. Set rate, clip speed, pitch, and existing-TTS policy
5. Select captions → **Tạo và gắn TTS**
6. Close the project in CapCut before write, then reopen CapCut to check the timeline

> [!IMPORTANT]
> Close the CapCut project before writing. CapDraft TTS keeps local backups, but keep your own backup of important drafts.

## How free CapCut TTS works here

```text
CapCut project captions
        │
        ▼
 CapDraft TTS  ──►  local CapCut TTS API + device.json  ──►  CapCut TTS voices
        │
        ▼
 audio attached back into the same draft timeline
```

- Voices come from CapCut’s catalog (listed online as `Voice.json` on this repo)
- Generation goes through your CapCut TTS API setup, **not** a paid cloud TTS provider
- The app does not sell voices or charge per character

## Voice catalog

Loaded **live in memory** from:

```text
https://raw.githubusercontent.com/KhoaDayy/CapDraft-TTS/refs/heads/main/Voice.json
```

- No local catalog file is required in the app package
- Change URL or click **Tải lại danh sách** in **Settings → Giọng đọc**
- To add/edit voices for everyone, update `Voice.json` on `main`

## Configuration

`config.json` is machine-local (gitignored). Copy from [`config.example.json`](config.example.json).

| Key | Purpose |
| --- | --- |
| `capcut_tts_path` | CapCut TTS API folder |
| `device_json_path` | `device.json` for CapCut TTS |
| `voice_catalog_url` | Raw URL of the voice list JSON |
| `ffmpeg_path` / `ffprobe_path` | Media tools |
| `tts_chunk_size` | Captions per batch |
| `tts_parallel_chunks` | Parallel batches |
| `tts_download_workers` | Parallel audio downloads |
| `tts_poll_interval_sec` | Poll interval while waiting for TTS |
| `cache_path` | Generated audio cache |
| `max_backups` | Draft backups to keep |

## Project layout

```text
main.py                 App entry
core/config.py          Config
core/capcut_tts.py      CapCut TTS API wrapper
core/capcut_project/    Read / patch / validate CapCut drafts
ui/                     Desktop UI
tests/                  Tests
Voice.json              Public voice list (fetched by URL)
docs/                   Translated READMEs
```

## Development

```powershell
python -m unittest discover -s tests -v
.\build-release.ps1 -Version 1.0.1
```

See [`CONTRIBUTING.md`](CONTRIBUTING.md) and [`CHANGELOG.md`](CHANGELOG.md).

## License / legal

- [`LICENSE`](LICENSE) (Apache-2.0)
- Independent project — **not** affiliated with CapCut / ByteDance
- You are responsible for CapCut terms, device/account rules, and content rights
- QFluentWidgets is GPLv3 for non-commercial use; review licenses before commercial redistribution
