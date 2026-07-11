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

- **120+ CapCut voices**
- **No extra paid voice API**
- Reads captions from your CapCut project and writes audio back into the same timeline
- Exports project captions directly to a standard `.srt` subtitle file
- No project copy, no manual drag-and-drop of audio files

> You still need CapCut Desktop and internet. “Free” means the app does not require a separate paid TTS service.

## Features

- Open a CapCut project folder or `draft_content.json`
- Caption list with search, select-all, empty / no-TTS / error filters
- Online voice catalog (~120+ voices) with language filter + search
- TTS rate, CapCut clip speed, pitch mode (follow speed / preserve pitch)
- Replace existing TTS or skip captions that already have TTS
- Optional audio cache and native CapCut head-alignment
- Parallel generation, cancel, progress log
- Backup + atomic save with rollback on failed write
- Vietnamese, English, Chinese, and Japanese UI
- Follow Windows, light, or dark theme with Windows Blue accent

## Requirements

- Windows 10/11
- CapCut Desktop + a project that already has captions
- FFprobe is optional and only used as a fallback for reading cached audio duration
- Internet connection

Source development needs [`uv`](https://docs.astral.sh/uv/); it installs the matching Python environment and locked dependencies automatically.

## Install

### Prebuilt (recommended)

1. Download `CapDraft-TTS-v1.1.1-windows-x64.zip` from [Releases](https://github.com/KhoaDayy/CapDraft-TTS/releases)
2. Unzip and run `CapDraft-TTS.exe`
3. Choose a CapCut project and start working; normal use does not require editing `config.json`.

```powershell
Get-FileHash .\CapDraft-TTS-v1.1.1-windows-x64.zip -Algorithm SHA256
# compare with the .sha256 file from the release
```

### From source

```powershell
git clone https://github.com/KhoaDayy/CapDraft-TTS.git
cd CapDraft-TTS
uv sync --group dev
Copy-Item config.example.json config.json
uv run python main.py
```

## Usage

1. **Chọn project** → CapCut project folder or `draft_content.json`
2. Review / filter captions
3. Use **Xuất SRT** to save every non-empty caption as a subtitle file, or continue with TTS
4. Pick language + voice (120+ CapCut voices from the online catalog)
5. Set rate, clip speed, pitch, and existing-TTS policy
6. Select captions → **Tạo và gắn TTS**
7. Close the project in CapCut before write, then reopen CapCut to check the timeline

> [!IMPORTANT]
> Close the CapCut project before writing. CapDraft TTS keeps local backups, but keep your own backup of important drafts.

## Development

```powershell
uv run python -m unittest discover -s tests -v
.\build-release.ps1 -Version 1.1.1
```

See [`CONTRIBUTING.md`](CONTRIBUTING.md) and [`CHANGELOG.md`](CHANGELOG.md).

## License / legal

- [`LICENSE`](LICENSE) (Apache-2.0)
- Independent project — **not** affiliated with CapCut / ByteDance
- You are responsible for CapCut terms, device/account rules, and content rights
- QFluentWidgets is GPLv3 for non-commercial use; review licenses before commercial redistribution
