<p align="center">
  <img src="assets/capdraft-tts-banner.png" alt="CapDraft TTS — 免费 CapCut TTS" width="100%">
</p>

<h1 align="center">CapDraft TTS</h1>

<p align="center">
  <a href="../README.md">English</a> ·
  <a href="README.vi.md">Tiếng Việt</a> ·
  <strong>中文</strong> ·
  <a href="README.ja.md">日本語</a>
</p>

<p align="center">
  从工程字幕<strong>免费生成 120+ CapCut TTS 音色</strong>，<br>
  并把音频写回同一个 CapCut 草稿时间线。
</p>

<p align="center">
  <a href="https://github.com/KhoaDayy/CapDraft-TTS/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/KhoaDayy/CapDraft-TTS?label=release"></a>
  <a href="https://github.com/KhoaDayy/CapDraft-TTS/stargazers"><img alt="Stars" src="https://img.shields.io/github/stars/KhoaDayy/CapDraft-TTS?style=social"></a>
  <a href="../LICENSE"><img alt="License" src="https://img.shields.io/github/license/KhoaDayy/CapDraft-TTS"></a>
  <a href="https://www.python.org/"><img alt="Python" src="https://img.shields.io/badge/python-3.10%2B-blue"></a>
  <a href="#运行要求"><img alt="Platform" src="https://img.shields.io/badge/platform-Windows-0078D6"></a>
</p>

<p align="center">
  <img alt="capcut" src="https://img.shields.io/badge/capcut-TTS-black">
  <img alt="text-to-speech" src="https://img.shields.io/badge/text--to--speech-120%2B%20voices-success">
  <img alt="free" src="https://img.shields.io/badge/TTS-free%20%28no%20paid%20API%29-brightgreen">
  <img alt="desktop" src="https://img.shields.io/badge/desktop-PySide6-orange">
  <img alt="video" src="https://img.shields.io/badge/video-editing-lightgrey">
</p>

---

## 为什么做这个

CapCut 本身就有大量 TTS 音色。CapDraft TTS 让你在字幕上直接使用它们，**无需付费第三方 TTS 服务**。

- **120+ CapCut 音色**（从 GitHub 在线拉取目录）
- **无 TTS 费用** — 走 CapCut 自有 TTS 路径（本地 CapCut TTS API + `device.json`）
- 读取工程字幕，并把音频写回同一时间线
- 不复制工程、不手动拖入音频

> 仍需 CapCut 桌面版、可用的 CapCut TTS API 环境，以及网络。“免费”指不额外购买 ElevenLabs、Azure 等语音 API。

## 功能

- 打开 CapCut 工程目录或 `draft_content.json`
- 字幕列表：搜索、全选、空行 / 无 TTS / 错误筛选
- 在线音色目录（约 120+），支持语言筛选与搜索
- TTS 语速、CapCut 片段速度、音高模式
- 替换已有 TTS，或跳过已有 TTS 的字幕
- 可选音频缓存与 CapCut 原生片头对齐
- 并行生成、取消任务、进度日志
- 备份 + 原子写入，失败可回滚
- Fluent 深色界面（UI 文案为越南语）

## 运行要求

- Windows 10/11
- CapCut 桌面版 + 已有字幕的工程
- 本地 CapCut TTS API，以及有效 `device.json`
- FFmpeg / FFprobe（在 `PATH` 中，或在设置里指定）
- 网络（音色目录 + CapCut TTS 请求）

从源码运行还需 Python 3.10+。

## 安装

### 预编译包（推荐）

1. 从 [Releases](https://github.com/KhoaDayy/CapDraft-TTS/releases) 下载 `CapDraft-TTS-v1.0.1-windows-x64.zip`
2. 解压并运行 `CapDraft-TTS.exe`
3. 打开 **设置**（齿轮），配置：
   - CapCut TTS API 目录
   - `device.json`
   - 若未加入 `PATH`，再配置 FFmpeg / FFprobe

```powershell
Get-FileHash .\CapDraft-TTS-v1.0.1-windows-x64.zip -Algorithm SHA256
# 与 release 中的 .sha256 文件对比
```

### 从源码运行

```powershell
git clone https://github.com/KhoaDayy/CapDraft-TTS.git
cd CapDraft-TTS
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install -r requirements.txt
Copy-Item config.example.json config.json
python main.py
```

## 使用方法

1. **Chọn project** → CapCut 工程目录或 `draft_content.json`
2. 检查 / 筛选字幕
3. 选择语言 + 音色（在线目录中 120+ CapCut 音色）
4. 设置语速、片段速度、音高、已有 TTS 策略
5. 勾选字幕 → **Tạo và gắn TTS**
6. 写入前先关闭 CapCut 中的工程，再重新打开检查时间线

> [!IMPORTANT]
> 写入前请关闭 CapCut 工程。应用会做本地备份，但重要草稿仍请自行备份。

## 免费 CapCut TTS 如何工作

```text
CapCut 工程字幕
        │
        ▼
 CapDraft TTS  ──►  本地 CapCut TTS API + device.json  ──►  CapCut TTS 音色
        │
        ▼
 音频写回同一草稿时间线
```

- 音色来自 CapCut 目录（本仓库 `Voice.json` 在线列出）
- 通过你的 CapCut TTS API 生成，**不是**付费云 TTS
- 应用不售卖音色、不按字符收费

## 音色目录

**内存实时加载**，来源：

```text
https://raw.githubusercontent.com/KhoaDayy/CapDraft-TTS/refs/heads/main/Voice.json
```

- 安装包无需附带本地目录文件
- 在 **设置 → Giọng đọc** 可改 URL 或点 **Tải lại danh sách** 重新拉取
- 要为所有人增改音色：更新 `main` 分支上的 `Voice.json`

## 配置

`config.json` 为本机配置（已 gitignore）。从 [`config.example.json`](../config.example.json) 复制。

| 键 | 用途 |
| --- | --- |
| `capcut_tts_path` | CapCut TTS API 目录 |
| `device_json_path` | CapCut TTS 的 `device.json` |
| `voice_catalog_url` | 音色列表 JSON 的 raw URL |
| `ffmpeg_path` / `ffprobe_path` | 媒体工具 |
| `tts_chunk_size` | 每批字幕数 |
| `tts_parallel_chunks` | 并行批次数 |
| `tts_download_workers` | 并行下载线程 |
| `tts_poll_interval_sec` | 等待 TTS 的轮询间隔 |
| `cache_path` | 生成音频缓存 |
| `max_backups` | 保留草稿备份数 |

## 项目结构

```text
main.py                 入口
core/config.py          配置
core/capcut_tts.py      CapCut TTS API 封装
core/capcut_project/    读写 / 校验 CapCut 草稿
ui/                     桌面 UI
tests/                  测试
Voice.json              公开音色列表（按 URL 拉取）
docs/                   多语言 README
```

## 开发

```powershell
python -m unittest discover -s tests -v
.\build-release.ps1 -Version 1.0.1
```

参见 [`CONTRIBUTING.md`](../CONTRIBUTING.md) 与 [`CHANGELOG.md`](../CHANGELOG.md)。

## 许可 / 法律声明

- [`LICENSE`](../LICENSE)（Apache-2.0）
- 独立项目 — **与** CapCut / ByteDance **无**隶属或背书关系
- 请自行遵守 CapCut 条款、账号/设备规则及内容版权
- QFluentWidgets 非商业用途为 GPLv3；商业分发前请核对许可证
