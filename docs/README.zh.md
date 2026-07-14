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

---

## 为什么做这个

CapCut 自带大量 TTS 音色。CapDraft TTS 让你在字幕上直接使用它们，**无需付费第三方 TTS**。

- **120+ CapCut 音色**
- 读取工程字幕并写回同一时间线
- 大型工程（约 5k 字幕）：虚拟化表格、更精简的草稿便于导出
- **拆成 2 份**可打开的 CapCut 工程 → 分别导出 → 用 **ffmpeg 合并视频**
- 导出 `.srt`

> 仍需 CapCut 桌面版与网络。“免费”指不额外购买语音 API。

## 功能

| 模块 | 说明 |
|------|------|
| **工程** | 打开工程目录或 `draft_content.json`（含 `Timelines/` 布局） |
| **字幕** | 约 5k 行虚拟表；搜索；全选；空行 / 无 TTS / 仅错误 |
| **音色** | 在线目录 ~120+，语言筛选与搜索 |
| **TTS** | 片段速度、音高、替换/跳过已有 TTS、缓存、原生片头对齐 |
| **流水线** | 并行生成、取消、日志、备份 + 原子写入 + 回滚 |
| **导出辅助** | **拆成 2 份** · **合并视频**（ffmpeg） · **导出 SRT** |
| **界面** | 越/英/中/日 · Windows 明/暗/自动主题 |

## 运行要求

- Windows 10/11
- CapCut 桌面版 + 已有字幕的工程
- 网络（TTS + 音色目录）
- **ffmpeg**（`PATH` 或 `ffmpeg_path`）— 仅 **合并视频** 需要
- **ffprobe** 可选

源码开发请使用 [`uv`](https://docs.astral.sh/uv/)。

## 安装

### 预编译包（推荐）

1. 从 [Releases](https://github.com/KhoaDayy/CapDraft-TTS/releases/latest) 下载最新 `CapDraft-TTS-v*-windows-x64.zip`
2. 解压运行 `CapDraft-TTS.exe`
3. 选择 CapCut 工程即可；一般无需改 `config.json`

```powershell
Get-FileHash .\CapDraft-TTS-v1.2.4-windows-x64.zip -Algorithm SHA256
```

### 从源码运行

```powershell
git clone https://github.com/KhoaDayy/CapDraft-TTS.git
cd CapDraft-TTS
uv sync --group dev
Copy-Item config.example.json config.json
uv run python main.py
```

可选配置见 `config.example.json`：`capcut_projects_path`、`ffmpeg_path`、性能与语言主题等。

## 使用方法

### 生成 TTS

1. **选择工程** → CapCut 目录或 `draft_content.json`
2. 检查 / 筛选字幕
3. 选择语言与音色
4. 设置速度、音高、已有 TTS 策略
5. 勾选字幕 → **生成并附加 TTS**
6. **写入前关闭 CapCut 中的工程**，再重新打开检查

### 大型工程 / CapCut 导出失败

1. 在 CapDraft TTS 中加载工程（已挂 TTS 亦可）
2. **拆成 2 份** → 在源工程旁生成 `_part1` / `_part2`
3. 完全退出并重启 CapCut，分别导出两半
4. **合并视频** → 按 part1、part2 顺序选择导出文件 → 保存一个 MP4

> [!IMPORTANT]
> 写入 TTS 或拆分前请关闭 CapCut（或工程）。应用会做 TTS 写入备份，重要草稿仍请自行备份。

### 导出 SRT

**导出 SRT** 将非空字幕保存为 `.srt`。

## 开发

```powershell
uv run python -m unittest discover -s tests -v
.\build-release.ps1 -Version 1.2.4
```

发布：推送 `v*` 标签触发 CI。详见 [`CHANGELOG.md`](../CHANGELOG.md)、[`CONTRIBUTING.md`](../CONTRIBUTING.md)。

## 许可 / 法律

- [`LICENSE`](../LICENSE)（Apache-2.0）
- 独立项目 — **非** CapCut / ByteDance 官方
- 请自行遵守 CapCut 条款与内容版权
- QFluentWidgets 非商业 GPLv3；商用分发前请核查许可证
