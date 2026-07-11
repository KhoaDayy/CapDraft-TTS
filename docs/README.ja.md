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

<p align="center">
  <img alt="capcut" src="https://img.shields.io/badge/capcut-TTS-black">
  <img alt="text-to-speech" src="https://img.shields.io/badge/text--to--speech-120%2B%20voices-success">
  <img alt="free" src="https://img.shields.io/badge/TTS-free%20%28no%20paid%20API%29-brightgreen">
  <img alt="desktop" src="https://img.shields.io/badge/desktop-PySide6-orange">
  <img alt="video" src="https://img.shields.io/badge/video-editing-lightgrey">
</p>

---

## なぜ作ったか

CapCut には多数の TTS 音声があります。CapDraft TTS は、それらをキャプションに使うためのツールで、**有料の第三者 TTS サービスは不要**です。

- **120+ CapCut 音声**（GitHub からライブ取得）
- **TTS 料金なし** — ローカル CapCut TTS API + `device.json` で CapCut 本体の TTS 経路を利用
- CapCut プロジェクトのキャプションを読み、同じタイムラインへ音声を書き戻し
- プロジェクト複製なし、音声の手動ドラッグ＆ドロップなし

> CapCut Desktop、動作する CapCut TTS API、インターネットは必要です。「無料」は ElevenLabs / Azure などの追加有料音声 API を使わない、という意味です。

## 機能

- CapCut プロジェクトフォルダまたは `draft_content.json` を開く
- キャプション一覧：検索、全選択、空行 / TTS なし / エラー絞り込み
- オンライン音声カタログ（約 120+）、言語フィルタと検索
- TTS 速度、CapCut クリップ速度、ピッチモード
- 既存 TTS の置換、または既存 TTS 付きキャプションのスキップ
- 任意の音声キャッシュと CapCut ネイティブ先頭アライン
- 並列生成、キャンセル、進捗ログ
- バックアップ + アトミック保存、失敗時ロールバック
- Fluent ダーク UI（UI 文言はベトナム語）

## 動作要件

- Windows 10/11
- CapCut Desktop + キャプション済みプロジェクト
- ローカル CapCut TTS API と有効な `device.json`
- FFmpeg / FFprobe（`PATH` または設定で指定）
- インターネット（音声カタログ + CapCut TTS 通信）

ソース実行時は Python 3.10+ も必要です。

## インストール

### ビルド済み（推奨）

1. [Releases](https://github.com/KhoaDayy/CapDraft-TTS/releases) から `CapDraft-TTS-v1.0.1-windows-x64.zip` をダウンロード
2. 展開して `CapDraft-TTS.exe` を実行
3. **設定**（歯車）で次を指定：
   - CapCut TTS API フォルダ
   - `device.json`
   - `PATH` に無い場合は FFmpeg / FFprobe

```powershell
Get-FileHash .\CapDraft-TTS-v1.0.1-windows-x64.zip -Algorithm SHA256
# リリースの .sha256 と比較して確認
```

### ソースから実行

```powershell
git clone https://github.com/KhoaDayy/CapDraft-TTS.git
cd CapDraft-TTS
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install -r requirements.txt
Copy-Item config.example.json config.json
python main.py
```

## 使い方

1. **Chọn project** → CapCut プロジェクトフォルダまたは `draft_content.json`
2. キャプションを確認 / 絞り込み
3. 言語 + 音声を選択（オンラインカタログの 120+ CapCut 音声）
4. 速度・クリップ速度・ピッチ・既存 TTS 方針を設定
5. キャプションを選択 → **Tạo và gắn TTS**
6. 書き込み前に CapCut でプロジェクトを閉じ、再オープンしてタイムラインを確認

> [!IMPORTANT]
> 書き込み前に CapCut プロジェクトを閉じてください。アプリはローカルバックアップを作りますが、重要なドラフトは別途バックアップしてください。

## 無料 CapCut TTS の流れ

```text
CapCut プロジェクトのキャプション
        │
        ▼
 CapDraft TTS  ──►  ローカル CapCut TTS API + device.json  ──►  CapCut TTS 音声
        │
        ▼
 同じドラフトのタイムラインへ音声を書き戻し
```

- 音声は CapCut カタログ由来（本リポジトリの `Voice.json` をオンライン公開）
- 生成はあなたの CapCut TTS API 経由で、**有料クラウド TTS ではない**
- アプリは音声販売や文字課金をしません

## 音声カタログ

次の URL から**メモリへ直接ロード**します：

```text
https://raw.githubusercontent.com/KhoaDayy/CapDraft-TTS/refs/heads/main/Voice.json
```

- アプリパッケージにローカルカタログは不要
- **設定 → Giọng đọc** で URL 変更、または **Tải lại danh sách** で再取得
- 全員向けに音声を追加/編集する場合は `main` の `Voice.json` を更新

## 設定

`config.json` はマシン固有（gitignore）。[`config.example.json`](../config.example.json) をコピーしてください。

| キー | 用途 |
| --- | --- |
| `capcut_tts_path` | CapCut TTS API フォルダ |
| `device_json_path` | CapCut TTS 用 `device.json` |
| `voice_catalog_url` | 音声リスト JSON の raw URL |
| `ffmpeg_path` / `ffprobe_path` | メディアツール |
| `tts_chunk_size` | 1 バッチあたりのキャプション数 |
| `tts_parallel_chunks` | 並列バッチ数 |
| `tts_download_workers` | 音声ダウンロード並列数 |
| `tts_poll_interval_sec` | TTS 待ちポーリング間隔 |
| `cache_path` | 生成音声キャッシュ |
| `max_backups` | ドラフトバックアップ保持数 |

## プロジェクト構成

```text
main.py                 エントリ
core/config.py          設定
core/capcut_tts.py      CapCut TTS API ラッパ
core/capcut_project/    CapCut ドラフトの読取 / パッチ / 検証
ui/                     デスクトップ UI
tests/                  テスト
Voice.json              公開音声リスト（URL 取得）
docs/                   多言語 README
```

## 開発

```powershell
python -m unittest discover -s tests -v
.\build-release.ps1 -Version 1.0.1
```

[`CONTRIBUTING.md`](../CONTRIBUTING.md) と [`CHANGELOG.md`](../CHANGELOG.md) を参照。

## Star history

[![Star History Chart](https://api.star-history.com/svg?repos=KhoaDayy/CapDraft-TTS&type=Date)](https://star-history.com/#KhoaDayy/CapDraft-TTS&Date)

## ライセンス / 法的注意

- [`LICENSE`](../LICENSE)（Apache-2.0）
- 独立プロジェクト — CapCut / ByteDance とは**無関係・非公認**
- CapCut 利用規約、端末/アカウント規則、コンテンツ著作権は利用者責任
- QFluentWidgets は非商用で GPLv3；商用配布前にライセンスを確認してください
