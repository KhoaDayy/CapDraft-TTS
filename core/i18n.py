"""Runtime translations for the desktop UI."""

from __future__ import annotations

SUPPORTED_LANGUAGES = {"vi": "Tiếng Việt", "en": "English", "zh": "中文", "ja": "日本語"}
_current_language = "vi"


def normalize_language(value: object) -> str:
    code = str(value or "vi").strip().lower().replace("_", "-").split("-", 1)[0]
    return code if code in SUPPORTED_LANGUAGES else "vi"


def set_language(language: object) -> str:
    global _current_language
    _current_language = normalize_language(language)
    return _current_language


def get_language() -> str:
    return _current_language


_VI = {
    "settings": "Cài đặt", "settings_description": "Tùy chỉnh giao diện, đường dẫn, giọng đọc và hiệu năng TTS.",
    "appearance": "Giao diện", "language": "Ngôn ngữ", "theme": "Chủ đề", "theme_auto": "Theo Windows", "theme_light": "Sáng", "theme_dark": "Tối",
    "restart_language": "Ngôn ngữ mới sẽ áp dụng sau khi mở lại ứng dụng.", "save_settings": "Lưu cài đặt", "cancel": "Hủy", "paths": "Đường dẫn", "voices": "Giọng đọc", "performance": "Hiệu năng",
    "choose": "Chọn", "choose_folder": "Chọn thư mục", "choose_file": "Chọn tệp", "reload_list": "Tải lại danh sách", "voice_catalog_url": "URL danh mục giọng",
    "captions_per_batch": "Caption mỗi lô", "parallel_batches": "Số lô song song", "audio_workers": "Luồng tải audio", "poll_interval": "Chu kỳ kiểm tra", "backup_count": "Số bản sao lưu", "seconds_suffix": " giây",
    "window_title": "CapDraft TTS — Tạo TTS cho project CapCut", "capcut_project": "Project CapCut", "choose_project": "Chọn project", "project_placeholder": "Chọn thư mục project hoặc draft_content.json…", "reload_project": "Tải lại project", "export_srt": "Xuất SRT", "no_project": "Chưa chọn project",
    "voice_settings": "Cài đặt giọng đọc", "all": "Tất cả", "search_voice": "Tìm giọng…", "language_label": "Ngôn ngữ", "voice_label": "Giọng đọc", "voice_speed": "Tốc độ giọng đọc", "pitch": "Cao độ", "existing_tts": "TTS đã tồn tại", "replace_tts": "Thay thế TTS cũ", "skip_existing_tts": "Bỏ qua caption đã có TTS",
    "advanced_options": "Tùy chọn nâng cao", "use_cache": "Dùng cache TTS", "align_tts": "Chống lệch đầu TTS (native CapCut)", "trim_start": "Cắt đầu", "fade_in": "Fade-in",
    "search_caption": "Tìm trong nội dung caption…", "select_all": "Chọn tất cả", "deselect_all": "Bỏ chọn", "hide_empty": "Ẩn dòng rỗng", "without_tts": "Chưa có TTS", "only_errors": "Chỉ lỗi", "selection": "Đã chọn {selected}/{total} · Hiển thị {visible}",
    "table_start": "Bắt đầu", "table_end": "Kết thúc", "table_content": "Nội dung", "table_has_tts": "Đã có TTS", "table_status": "Trạng thái", "table_duration": "Thời lượng", "table_error": "Lỗi",
    "ready": "Sẵn sàng", "generating": "Đang tạo…", "cancelling": "Đang hủy…", "generate_attach": "Tạo và gắn TTS", "app_settings": "Cài đặt ứng dụng", "clear_log": "Xóa nhật ký", "copy_log": "Sao chép nhật ký", "save_log": "Lưu nhật ký ra file", "empty_filter": "Không có caption khớp bộ lọc hiện tại.", "yes": "Có", "no": "Không", "empty": "Rỗng", "has_tts": "Đã có TTS", "error": "Lỗi",
}

_EN = {
    "settings": "Settings", "settings_description": "Customize appearance, paths, voices, and TTS performance.", "appearance": "Appearance", "language": "Language", "theme": "Theme", "theme_auto": "Use Windows setting", "theme_light": "Light", "theme_dark": "Dark", "restart_language": "The new language will apply after restarting the app.", "save_settings": "Save settings", "cancel": "Cancel", "paths": "Paths", "voices": "Voices", "performance": "Performance", "choose": "Browse", "choose_folder": "Choose folder", "choose_file": "Choose file", "reload_list": "Reload list", "voice_catalog_url": "Voice catalog URL", "captions_per_batch": "Captions per batch", "parallel_batches": "Parallel batches", "audio_workers": "Audio download workers", "poll_interval": "Polling interval", "backup_count": "Backups to keep", "seconds_suffix": " seconds",
    "window_title": "CapDraft TTS — TTS for CapCut projects", "capcut_project": "CapCut project", "choose_project": "Choose project", "project_placeholder": "Choose a project folder or draft_content.json…", "reload_project": "Reload project", "export_srt": "Export SRT", "no_project": "No project selected", "voice_settings": "Voice settings", "all": "All", "search_voice": "Search voices…", "language_label": "Language", "voice_label": "Voice", "voice_speed": "Voice speed", "pitch": "Pitch", "existing_tts": "Existing TTS", "replace_tts": "Replace existing TTS", "skip_existing_tts": "Skip captions with TTS", "advanced_options": "Advanced options", "use_cache": "Use TTS cache", "align_tts": "Prevent TTS head offset (native CapCut)", "trim_start": "Head trim", "fade_in": "Fade-in", "search_caption": "Search caption text…", "select_all": "Select all", "deselect_all": "Deselect all", "hide_empty": "Hide empty", "without_tts": "Without TTS", "only_errors": "Errors only", "selection": "Selected {selected}/{total} · Showing {visible}", "table_start": "Start", "table_end": "End", "table_content": "Content", "table_has_tts": "Has TTS", "table_status": "Status", "table_duration": "Duration", "table_error": "Error", "ready": "Ready", "generating": "Generating…", "cancelling": "Cancelling…", "generate_attach": "Generate and attach TTS", "app_settings": "App settings", "clear_log": "Clear log", "copy_log": "Copy log", "save_log": "Save log to file", "empty_filter": "No captions match the current filters.", "yes": "Yes", "no": "No", "empty": "Empty", "has_tts": "Has TTS", "error": "Error",
}

_ZH = dict(_EN, **{
    "settings": "设置", "settings_description": "自定义外观、路径、语音和 TTS 性能。", "appearance": "外观", "language": "语言", "theme": "主题", "theme_auto": "跟随 Windows", "theme_light": "浅色", "theme_dark": "深色", "restart_language": "新语言将在重新打开应用后生效。", "save_settings": "保存设置", "cancel": "取消", "paths": "路径", "voices": "语音", "performance": "性能", "choose": "选择", "choose_folder": "选择文件夹", "choose_file": "选择文件", "reload_list": "重新加载列表", "window_title": "CapDraft TTS — CapCut 项目语音", "capcut_project": "CapCut 项目", "choose_project": "选择项目", "project_placeholder": "选择项目文件夹或 draft_content.json…", "reload_project": "重新加载项目", "export_srt": "导出 SRT", "no_project": "未选择项目", "voice_settings": "语音设置", "all": "全部", "search_voice": "搜索语音…", "language_label": "语言", "voice_label": "语音", "voice_speed": "语速", "pitch": "音调", "existing_tts": "已有 TTS", "replace_tts": "替换已有 TTS", "skip_existing_tts": "跳过已有 TTS 的字幕", "advanced_options": "高级选项", "use_cache": "使用 TTS 缓存", "align_tts": "修正 TTS 开头偏移（CapCut 原生）", "trim_start": "开头裁剪", "search_caption": "搜索字幕内容…", "select_all": "全选", "deselect_all": "取消全选", "hide_empty": "隐藏空字幕", "without_tts": "无 TTS", "only_errors": "仅错误", "selection": "已选 {selected}/{total} · 显示 {visible}", "table_start": "开始", "table_end": "结束", "table_content": "内容", "table_has_tts": "已有 TTS", "table_status": "状态", "table_duration": "时长", "table_error": "错误", "ready": "就绪", "generating": "正在生成…", "cancelling": "正在取消…", "generate_attach": "生成并附加 TTS", "app_settings": "应用设置", "clear_log": "清空日志", "copy_log": "复制日志", "save_log": "保存日志", "empty_filter": "没有符合当前筛选条件的字幕。", "yes": "是", "no": "否", "empty": "空", "has_tts": "已有 TTS", "error": "错误",
})

_JA = dict(_EN, **{
    "settings": "設定", "settings_description": "外観、パス、音声、TTS 性能を設定します。", "appearance": "外観", "language": "言語", "theme": "テーマ", "theme_auto": "Windows に合わせる", "theme_light": "ライト", "theme_dark": "ダーク", "restart_language": "新しい言語はアプリの再起動後に適用されます。", "save_settings": "設定を保存", "cancel": "キャンセル", "paths": "パス", "voices": "音声", "performance": "性能", "choose": "選択", "choose_folder": "フォルダーを選択", "choose_file": "ファイルを選択", "reload_list": "リストを再読込", "window_title": "CapDraft TTS — CapCut プロジェクト用 TTS", "capcut_project": "CapCut プロジェクト", "choose_project": "プロジェクトを選択", "project_placeholder": "プロジェクトフォルダーまたは draft_content.json を選択…", "reload_project": "プロジェクトを再読込", "export_srt": "SRT を出力", "no_project": "プロジェクト未選択", "voice_settings": "音声設定", "all": "すべて", "search_voice": "音声を検索…", "language_label": "言語", "voice_label": "音声", "voice_speed": "音声速度", "pitch": "ピッチ", "existing_tts": "既存の TTS", "replace_tts": "既存の TTS を置換", "skip_existing_tts": "TTS 付き字幕をスキップ", "advanced_options": "詳細オプション", "use_cache": "TTS キャッシュを使用", "align_tts": "TTS 先頭のずれを補正（CapCut ネイティブ）", "trim_start": "先頭トリム", "search_caption": "字幕内容を検索…", "select_all": "すべて選択", "deselect_all": "選択解除", "hide_empty": "空行を非表示", "without_tts": "TTS なし", "only_errors": "エラーのみ", "selection": "選択 {selected}/{total} · 表示 {visible}", "table_start": "開始", "table_end": "終了", "table_content": "内容", "table_has_tts": "TTS あり", "table_status": "状態", "table_duration": "長さ", "table_error": "エラー", "ready": "準備完了", "generating": "生成中…", "cancelling": "キャンセル中…", "generate_attach": "TTS を生成して追加", "app_settings": "アプリ設定", "clear_log": "ログを消去", "copy_log": "ログをコピー", "save_log": "ログを保存", "empty_filter": "現在のフィルターに一致する字幕はありません。", "yes": "はい", "no": "いいえ", "empty": "空", "has_tts": "TTS あり", "error": "エラー",
})

_CATALOGS = {"vi": _VI, "en": _EN, "zh": _ZH, "ja": _JA}


def translate(key: str, *, language: object | None = None, **values) -> str:
    lang = normalize_language(language if language is not None else _current_language)
    text = _CATALOGS[lang].get(key, _VI.get(key, key))
    return text.format(**values) if values else text


def tr(key: str, **values) -> str:
    return translate(key, **values)
