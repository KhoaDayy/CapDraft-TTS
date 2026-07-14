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
  <img alt="split-merge" src="https://img.shields.io/badge/export-split%20%2B%20ffmpeg%20merge-blueviolet">
</p>

---

## Why this exists

CapCut already has many TTS voices. CapDraft TTS lets you use them on your captions **without paid third-party TTS services**.

- **120+ CapCut voices**
- **No extra paid voice API**
- Reads captions from your CapCut project and writes audio back into the same timeline
- Handles large projects (thousands of captions) with a virtualized table and slimmer drafts for export
- **Split** a huge project into 2 CapCut-openable halves, export each in CapCut, then **merge** the videos with ffmpeg
- Exports captions to `.srt`
- No project copy for normal TTS attach — no manual drag-and-drop of audio files

> You still need CapCut Desktop and internet. “Free” means the app does not require a separate paid TTS service.

## Features

| Area | What you get |
|------|----------------|
| **Project** | Open CapCut project folder or `draft_content.json` (incl. modern `Timelines/` layout) |
| **Captions** | Virtualized table for ~5k rows; search; select all; hide empty / without TTS / errors only |
| **Voices** | Online catalog (~120+), language filter + search |
| **TTS** | Clip speed, pitch mode, replace or skip existing TTS, optional cache, native head trim/fade |
| **Pipeline** | Parallel generation, cancel, throttled progress log, backup + atomic write with rollback |
| **Export helpers** | **Split in 2** CapCut-safe projects · **Merge videos** (ffmpeg stream-copy, re-encode fallback) · **Export SRT** |
| **UI** | VI / EN / ZH / JA · Windows light/dark/auto theme |

## Requirements

- **Windows** 10/11
- **CapCut Desktop** + a project that already has captions
- **Internet** (TTS + voice catalog)
- **ffmpeg** on `PATH` (or set `ffmpeg_path` in config) — only needed for **Merge videos**
- **ffprobe** optional — fallback for cached audio duration

Source development needs [`uv`](https://docs.astral.sh/uv/).

## Install

### Prebuilt (recommended)

1. Download the latest `CapDraft-TTS-v*-windows-x64.zip` from [Releases](https://github.com/KhoaDayy/CapDraft-TTS/releases/latest)
2. Unzip and run `CapDraft-TTS.exe`
3. Choose a CapCut project and start; normal use does not require editing `config.json`

```powershell
Get-FileHash .\CapDraft-TTS-v1.2.4-windows-x64.zip -Algorithm SHA256
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

Optional `config.json` keys (see `config.example.json`):

| Key | Purpose |
|-----|---------|
| `capcut_projects_path` | Default folder when browsing projects (e.g. CapCut drafts or AutoVideo `outputs`) |
| `ffmpeg_path` | Path to `ffmpeg` for video merge |
| `ffprobe_path` | Path to `ffprobe` |
| `tts_chunk_size` / `tts_parallel_chunks` / `tts_download_workers` | Performance |
| `language` / `theme_mode` | UI |

## Usage

### Generate TTS

1. **Choose project** → CapCut project folder or `draft_content.json`
2. Review / filter captions
3. Pick language + voice
4. Set clip speed, pitch, existing-TTS policy
5. Select captions → **Generate and attach TTS**
6. **Close the project in CapCut** before write, then reopen CapCut to check the timeline

### Large projects that fail CapCut export

When CapCut export is too heavy (e.g. multi-hour fail on a big timeline):

1. Load the project in CapDraft TTS (after TTS is attached is fine)
2. Click **Split in 2** → creates `<name>_part1` and `<name>_part2` next to the source (source is not modified)
3. Fully quit CapCut, reopen, open each part → **Export** video
4. Click **Merge videos** → pick part1 then part2 exports → save one MP4 (ffmpeg; stream-copy when possible)

> [!IMPORTANT]
> Close CapCut (or at least the project) before TTS write or split. CapDraft keeps local backups for TTS writes, but keep your own backup of important drafts.

### Export SRT

**Export SRT** saves non-empty captions from the loaded project to a standard `.srt` file.

## Development

```powershell
uv run python -m unittest discover -s tests -v
.\build-release.ps1 -Version 1.2.4
```

Releases: push a `v*` tag (or run **Release** workflow_dispatch). CI builds Windows zip + publishes GitHub Release (see `.github/workflows/release.yml`).

See [`CONTRIBUTING.md`](CONTRIBUTING.md) and [`CHANGELOG.md`](CHANGELOG.md).

## License / legal

- [`LICENSE`](LICENSE) (Apache-2.0)
- Independent project — **not** affiliated with CapCut / ByteDance
- You are responsible for CapCut terms, device/account rules, and content rights
- QFluentWidgets is GPLv3 for non-commercial use; review licenses before commercial redistribution
