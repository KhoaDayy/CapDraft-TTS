<p align="center">
  <img src="assets/capdraft-tts-banner.png" alt="CapDraft TTS — 無料 CapCut TTS" width="100%">
</p>

<h1 align="center">CapDraft TTS</h1>

<p align="center">
  <a href="../README.md">English</a> ·
  <a href="README.vi.md">Tiếng Việt</a> ·
  <a href="README.zh.md">中文</a> ·
  <strong>日本語</strong>
</p>

<p align="center">
  プロジェクトのキャプションから<strong>120+ の CapCut TTS 音声を無料生成</strong>し、<br>
  同じ CapCut ドラフトのタイムラインへ音声を戻します。
</p>

<p align="center">
  <a href="https://github.com/KhoaDayy/CapDraft-TTS/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/KhoaDayy/CapDraft-TTS?label=release"></a>
  <a href="https://github.com/KhoaDayy/CapDraft-TTS/stargazers"><img alt="Stars" src="https://img.shields.io/github/stars/KhoaDayy/CapDraft-TTS?style=social"></a>
  <a href="../LICENSE"><img alt="License" src="https://img.shields.io/github/license/KhoaDayy/CapDraft-TTS"></a>
  <a href="https://www.python.org/"><img alt="Python" src="https://img.shields.io/badge/python-3.10%2B-blue"></a>
  <a href="#動作要件"><img alt="Platform" src="https://img.shields.io/badge/platform-Windows-0078D6"></a>
</p>

---

## なぜ作ったか

CapCut には多数の TTS 音声があります。CapDraft TTS はそれらをキャプションに使うためのツールで、**有料の第三者 TTS は不要**です。

- **120+ CapCut 音声**
- キャプションを読み、同じタイムラインへ音声を書き戻し
- 大規模プロジェクト（約 5k 行）：仮想化テーブル、書き出しやすい薄いドラフト
- プロジェクトを **2 つに分割** → CapCut で各半を書き出し → **ffmpeg で動画結合**
- `.srt` 書き出し

> CapCut Desktop とインターネットは必要です。「無料」は追加の有料音声 API を使わない、という意味です。

## 機能

| 領域 | 内容 |
|------|------|
| **プロジェクト** | フォルダまたは `draft_content.json`（`Timelines/` 含む） |
| **キャプション** | 約 5k 行の仮想テーブル、検索、全選択、空 / TTS なし / エラー |
| **音声** | オンラインカタログ ~120+、言語フィルタと検索 |
| **TTS** | クリップ速度、ピッチ、既存 TTS の置換/スキップ、キャッシュ、先頭アライン |
| **パイプライン** | 並列生成、キャンセル、ログ、バックアップ + アトミック保存 + ロールバック |
| **書き出し補助** | **2つに分割** · **動画を結合**（ffmpeg） · **SRT を出力** |
| **UI** | 越/英/中/日 · Windows ライト/ダーク/自動 |

## 動作要件

- Windows 10/11
- CapCut Desktop + キャプション済みプロジェクト
- インターネット（TTS + カタログ）
- **ffmpeg**（`PATH` または `ffmpeg_path`）— **動画結合**のみ
- **ffprobe** は任意

ソース開発は [`uv`](https://docs.astral.sh/uv/) を使用。

## インストール

### ビルド済み（推奨）

1. [Releases](https://github.com/KhoaDayy/CapDraft-TTS/releases/latest) から最新の `CapDraft-TTS-v*-windows-x64.zip` を取得
2. 展開して `CapDraft-TTS.exe` を実行
3. CapCut プロジェクトを選ぶだけ（通常 `config.json` 不要）

```powershell
Get-FileHash .\CapDraft-TTS-v1.2.4-windows-x64.zip -Algorithm SHA256
```

### ソースから実行

```powershell
git clone https://github.com/KhoaDayy/CapDraft-TTS.git
cd CapDraft-TTS
uv sync --group dev
Copy-Item config.example.json config.json
uv run python main.py
```

任意設定は `config.example.json` を参照（`capcut_projects_path`、`ffmpeg_path` など）。

## 使い方

### TTS 生成

1. **プロジェクトを選択** → CapCut フォルダまたは `draft_content.json`
2. キャプションを確認 / 絞り込み
3. 言語 + 音声を選択
4. 速度・ピッチ・既存 TTS 方針を設定
5. 選択 → **TTS を生成して追加**
6. **書き込み前に CapCut でプロジェクトを閉じ**、再オープンして確認

### 大規模 / CapCut 書き出し失敗時

1. CapDraft TTS でプロジェクトを読み込み（TTS 済みでも可）
2. **2つに分割** → ソース横に `_part1` / `_part2` を作成
3. CapCut を完全終了してから各 half を書き出し
4. **動画を結合** → part1 → part2 の順で選択 → 1 本の MP4 を保存

> [!IMPORTANT]
> TTS 書き込みや分割の前に CapCut（またはプロジェクト）を閉じてください。TTS 書き込み時はローカルバックアップがありますが、重要ドラフトは別途バックアップしてください。

### SRT 出力

**SRT を出力**で非空キャプションを `.srt` に保存します。

## 開発

```powershell
uv run python -m unittest discover -s tests -v
.\build-release.ps1 -Version 1.2.4
```

リリース：`v*` タグ push で CI。詳細は [`CHANGELOG.md`](../CHANGELOG.md)、[`CONTRIBUTING.md`](../CONTRIBUTING.md)。

## ライセンス / 法務

- [`LICENSE`](../LICENSE)（Apache-2.0）
- 独立プロジェクト — CapCut / ByteDance **非公式**
- CapCut 利用規約とコンテンツ権利は利用者の責任
- QFluentWidgets は非商用 GPLv3。商用配布前にライセンスを確認
