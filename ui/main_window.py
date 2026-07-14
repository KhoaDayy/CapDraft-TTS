"""CapCut Project TTS Generator — desktop workbench UI."""

from __future__ import annotations

import sys
from enum import Enum, auto
from pathlib import Path
from typing import Optional

from PySide6.QtCore import Qt, QThread, QTimer, Signal
from PySide6.QtGui import QColor, QFont, QPalette, QTextCursor
from PySide6.QtWidgets import (
    QAbstractItemView,
    QApplication,
    QFileDialog,
    QGridLayout,
    QGroupBox,
    QHBoxLayout,
    QHeaderView,
    QLabel,
    QMainWindow,
    QMessageBox,
    QSizePolicy,
    QSplitter,
    QStyleFactory,
    QTableView,
    QVBoxLayout,
    QWidget,
)
from qfluentwidgets import (
    CheckBox,
    ComboBox,
    DoubleSpinBox,
    FluentIcon,
    InfoBar,
    InfoBarPosition,
    LineEdit,
    PrimaryPushButton,
    ProgressBar,
    PushButton,
    TextEdit,
    Theme,
    ToolButton,
    isDarkTheme,
    setTheme,
    setThemeColor,
)

try:
    from qfluentwidgets import SearchLineEdit as _SearchLineEdit
except ImportError:  # older qfluentwidgets
    _SearchLineEdit = None  # type: ignore[misc, assignment]

# Ensure project root on path
ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from core.capcut_project.models import (  # noqa: E402
    GenerationLogEvent,
    GenerationResult,
    ToneModifyMode,
)
from core.capcut_project.native_audio_alignment import NativeAudioAlignmentSettings  # noqa: E402
from core.capcut_project.tts_project_service import CapCutProjectTtsService  # noqa: E402
from core.config import AppConfig  # noqa: E402
from core.i18n import set_language, tr  # noqa: E402
from core.logger import logger  # noqa: E402
from ui.caption_table_model import CaptionFilterProxy, CaptionTableModel  # noqa: E402
from ui.settings_dialog import SettingsDialog  # noqa: E402


class UiState(Enum):
    IDLE_NO_PROJECT = auto()
    IDLE_READY = auto()
    GENERATING = auto()
    CANCELLING = auto()


class GenerateWorker(QThread):
    log_event = Signal(object)
    progress = Signal(float, str)
    item_done = Signal(int, dict)
    finished_ok = Signal(object)
    failed = Signal(str)

    def __init__(
        self,
        service: CapCutProjectTtsService,
        selected_ids: list[str],
        voice_type: str,
        resource_id: str,
        voice_display_name: str,
        tts_rate: float,
        clip_speed: float,
        tone_mode: ToneModifyMode,
        existing_mode: str,
        use_cache: bool,
        alignment: NativeAudioAlignmentSettings | None = None,
    ):
        super().__init__()
        self.service = service
        self.selected_ids = selected_ids
        self.voice_type = voice_type
        self.resource_id = resource_id
        self.voice_display_name = voice_display_name
        self.tts_rate = tts_rate
        self.clip_speed = clip_speed
        self.tone_mode = tone_mode
        self.existing_mode = existing_mode
        self.use_cache = use_cache
        self.alignment = alignment or NativeAudioAlignmentSettings()
        self._cancel = False

    def cancel(self):
        self._cancel = True

    def run(self):
        try:
            def log_cb(ev: GenerationLogEvent):
                self.log_event.emit(ev)

            def prog_cb(p: float, msg: str):
                self.progress.emit(p, msg)

            def item_cb(idx: int, res: dict):
                self.item_done.emit(idx, res)

            result = self.service.generate_and_attach(
                selected_caption_ids=self.selected_ids,
                voice_type=self.voice_type,
                resource_id=self.resource_id,
                voice_display_name=self.voice_display_name,
                tts_rate=self.tts_rate,
                capcut_clip_speed=self.clip_speed,
                tone_modify_mode=self.tone_mode,
                existing_tts_mode=self.existing_mode,
                progress_callback=prog_cb,
                item_completed_callback=item_cb,
                log_callback=log_cb,
                is_cancelled_callback=lambda: self._cancel,
                use_cache=self.use_cache,
                alignment_settings=self.alignment,
            )
            self.finished_ok.emit(result)
        except Exception as e:
            logger.exception("Generate worker failed")
            self.failed.emit(str(e))


class SplitWorker(QThread):
    progress = Signal(float, str)
    finished_ok = Signal(object)
    failed = Signal(str)

    def __init__(
        self,
        source_project_dir: Path,
        source_draft: dict,
        source_draft_path: Path,
        caption_edges_us: list[int] | None = None,
    ):
        super().__init__()
        self.source_project_dir = source_project_dir
        self.source_draft = source_draft
        self.source_draft_path = source_draft_path
        self.caption_edges_us = caption_edges_us or []
        self._cancel = False

    def cancel(self):
        self._cancel = True

    def run(self):
        try:
            from core.capcut_project.project_split import split_project

            def prog_cb(p: float, msg: str):
                self.progress.emit(p, msg)

            result = split_project(
                source_project_dir=self.source_project_dir,
                source_draft=self.source_draft,
                source_draft_path=self.source_draft_path,
                caption_edges_us=self.caption_edges_us,
                progress_callback=prog_cb,
            )
            self.finished_ok.emit(result)
        except Exception as e:
            logger.exception("Split worker failed")
            self.failed.emit(str(e))


class MergeWorker(QThread):
    progress = Signal(float, str)
    finished_ok = Signal(object)
    failed = Signal(str)

    def __init__(self, inputs: list[str], output: str, ffmpeg_path: str | None = None):
        super().__init__()
        self.inputs = inputs
        self.output = output
        self.ffmpeg_path = ffmpeg_path
        self._cancel = False

    def cancel(self):
        self._cancel = True

    def run(self):
        try:
            from core.capcut_project.video_merge import merge_videos

            def prog_cb(p: float, msg: str):
                self.progress.emit(p, msg)

            result = merge_videos(
                self.inputs,
                self.output,
                ffmpeg_path=self.ffmpeg_path,
                progress_callback=prog_cb,
            )
            self.finished_ok.emit(result)
        except Exception as e:
            logger.exception("Merge worker failed")
            self.failed.emit(str(e))


class MainWindow(QMainWindow):
    def __init__(self):
        super().__init__()
        self.config = AppConfig()
        set_language(self.config.language)
        self._apply_theme()
        self.setWindowTitle(tr("window_title"))
        self.setMinimumSize(1024, 720)
        self.resize(1280, 860)
        self.service = CapCutProjectTtsService()
        self.worker: Optional[QThread] = None
        self._captions = []
        self._voices = []
        self._closing = False
        self._ui_state = UiState.IDLE_NO_PROJECT
        self._advanced_open = False
        self._caption_model = CaptionTableModel(self)
        self._caption_proxy = CaptionFilterProxy(self)
        self._caption_proxy.setSourceModel(self._caption_model)
        # Batch per-item table updates so 5k captions don't flood the UI thread.
        self._pending_item_updates: dict[int, dict] = {}
        self._item_flush_timer = QTimer(self)
        self._item_flush_timer.setSingleShot(True)
        self._item_flush_timer.setInterval(80)
        self._item_flush_timer.timeout.connect(self._flush_item_updates)
        # Debounce caption search/filter (textChanged is per keystroke).
        self._filter_timer = QTimer(self)
        self._filter_timer.setSingleShot(True)
        self._filter_timer.setInterval(120)
        self._filter_timer.timeout.connect(self._filter_table)
        self._init_ui()
        self._connect_ui_signals()
        self._apply_ui_state(UiState.IDLE_NO_PROJECT)
        self._load_voices()

    @staticmethod
    def _apply_theme():
        mode = AppConfig().theme_mode
        setTheme({"light": Theme.LIGHT, "dark": Theme.DARK}.get(mode, Theme.AUTO))
        setThemeColor("#0078D4")
        # Fluent only skins its own widgets; native Qt chrome stays system-colored
        # unless we push a palette (Fusion makes light/dark palettes reliable).
        MainWindow._apply_native_palette()

    @staticmethod
    def _apply_native_palette():
        app = QApplication.instance()
        if app is None:
            return
        if app.style().objectName().lower() != "fusion":
            fusion = QStyleFactory.create("Fusion")
            if fusion is not None:
                app.setStyle(fusion)

        dark = isDarkTheme()
        pal = QPalette()
        if dark:
            window = QColor(32, 32, 32)
            base = QColor(45, 45, 45)
            alt = QColor(39, 39, 39)
            text = QColor(240, 240, 240)
            disabled = QColor(140, 140, 140)
            highlight = QColor(0, 120, 212)
            button = QColor(55, 55, 55)
        else:
            window = QColor(243, 243, 243)
            base = QColor(255, 255, 255)
            alt = QColor(249, 249, 249)
            text = QColor(28, 28, 28)
            disabled = QColor(140, 140, 140)
            highlight = QColor(0, 120, 212)
            button = QColor(251, 251, 251)

        pal.setColor(QPalette.Window, window)
        pal.setColor(QPalette.WindowText, text)
        pal.setColor(QPalette.Base, base)
        pal.setColor(QPalette.AlternateBase, alt)
        pal.setColor(QPalette.Text, text)
        pal.setColor(QPalette.Button, button)
        pal.setColor(QPalette.ButtonText, text)
        pal.setColor(QPalette.ToolTipBase, base)
        pal.setColor(QPalette.ToolTipText, text)
        pal.setColor(QPalette.BrightText, QColor(255, 255, 255))
        pal.setColor(QPalette.Highlight, highlight)
        pal.setColor(QPalette.HighlightedText, QColor(255, 255, 255))
        pal.setColor(QPalette.Link, highlight)
        pal.setColor(QPalette.PlaceholderText, disabled)
        for group in (QPalette.Disabled,):
            pal.setColor(group, QPalette.WindowText, disabled)
            pal.setColor(group, QPalette.Text, disabled)
            pal.setColor(group, QPalette.ButtonText, disabled)
            pal.setColor(group, QPalette.Highlight, QColor(100, 100, 100) if dark else QColor(180, 180, 180))
        app.setPalette(pal)

    # ------------------------------------------------------------------
    # UI builders
    # ------------------------------------------------------------------
    def _init_ui(self):
        root = QWidget()
        self.setCentralWidget(root)
        layout = QVBoxLayout(root)
        layout.setContentsMargins(12, 10, 12, 10)
        layout.setSpacing(8)

        layout.addWidget(self._build_project_bar())
        layout.addWidget(self.lbl_info)
        layout.addWidget(self._build_voice_settings())
        layout.addWidget(self._build_advanced_panel())

        self.main_split = self._build_workspace_split()
        layout.addWidget(self.main_split, 1)
        layout.addLayout(self._build_footer())

    def _muted_label(self, text: str = "") -> QLabel:
        lbl = QLabel(text)
        lbl.setForegroundRole(QPalette.ColorRole.PlaceholderText)
        return lbl

    def _build_project_bar(self) -> QWidget:
        box = QGroupBox(tr("capcut_project"))
        row = QHBoxLayout(box)
        row.setContentsMargins(10, 8, 10, 8)
        row.setSpacing(8)

        self.ed_project = LineEdit()
        self.ed_project.setPlaceholderText(tr("project_placeholder"))
        self.ed_project.setReadOnly(True)
        self.ed_project.setClearButtonEnabled(False)
        self.ed_project.setMinimumWidth(280)

        self.btn_browse = PushButton(FluentIcon.FOLDER, tr("choose_project"))
        self.btn_browse.setToolTip("Chọn draft_content.json hoặc thư mục project CapCut")

        self.btn_reload = ToolButton(FluentIcon.SYNC)
        self.btn_reload.setToolTip(tr("reload_project"))

        self.btn_export_srt = PushButton(tr("export_srt"))
        self.btn_export_srt.setToolTip("Xuất toàn bộ caption trong project thành file phụ đề .srt")

        self.btn_split = PushButton(tr("split_project"))
        self.btn_split.setToolTip(
            "Tách project thành 2 nửa theo timeline (part1/part2) để export CapCut nhẹ hơn."
        )
        self.btn_merge = PushButton(tr("merge_videos"))
        self.btn_merge.setToolTip(
            "Ghép 2 video đã export (ffmpeg stream-copy, fallback re-encode)."
        )

        row.addWidget(self.ed_project, 1)
        row.addWidget(self.btn_browse)
        row.addWidget(self.btn_reload)
        row.addWidget(self.btn_export_srt)
        row.addWidget(self.btn_split)
        row.addWidget(self.btn_merge)

        self.lbl_info = self._muted_label(tr("no_project"))
        self.lbl_info.setWordWrap(True)
        self.lbl_info.setTextInteractionFlags(Qt.TextSelectableByMouse)
        font = self.lbl_info.font()
        font.setPointSize(max(9, font.pointSize() - 1))
        self.lbl_info.setFont(font)
        return box

    def _field_label(self, text: str) -> QLabel:
        lbl = QLabel(text)
        lbl.setAlignment(Qt.AlignRight | Qt.AlignVCenter)
        return lbl

    def _build_voice_settings(self) -> QWidget:
        box = QGroupBox(tr("voice_settings"))
        grid = QGridLayout(box)
        grid.setContentsMargins(10, 10, 10, 10)
        grid.setHorizontalSpacing(10)
        grid.setVerticalSpacing(8)
        # Labels stay compact; voice column gets the free width.
        grid.setColumnMinimumWidth(0, 96)
        grid.setColumnMinimumWidth(2, 72)
        grid.setColumnStretch(1, 1)
        grid.setColumnStretch(3, 2)

        self.cmb_lang = ComboBox()
        self.cmb_lang.setMinimumWidth(120)
        self.cmb_lang.setSizePolicy(QSizePolicy.Preferred, QSizePolicy.Fixed)

        self.cmb_voice = ComboBox()
        self.cmb_voice.setMinimumWidth(200)
        self.cmb_voice.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Fixed)

        self.ed_voice_search = LineEdit()
        self.ed_voice_search.setPlaceholderText(tr("search_voice"))
        self.ed_voice_search.setClearButtonEnabled(True)
        self.ed_voice_search.setFixedWidth(150)

        voice_wrap = QWidget()
        voice_row = QHBoxLayout(voice_wrap)
        voice_row.setSpacing(6)
        voice_row.setContentsMargins(0, 0, 0, 0)
        voice_row.addWidget(self.cmb_voice, 1)
        voice_row.addWidget(self.ed_voice_search)

        self.lbl_voice_adv = self._muted_label("")
        small = self.lbl_voice_adv.font()
        small.setPointSize(max(8, small.pointSize() - 1))
        self.lbl_voice_adv.setFont(small)
        self.lbl_voice_adv.setWordWrap(False)
        self.lbl_voice_adv.setTextInteractionFlags(Qt.TextSelectableByMouse)

        cfg = AppConfig()
        try:
            default_clip = float(cfg.get("default_clip_speed") or 1.0) or 1.0
        except Exception:
            default_clip = 1.0

        # capcut_clip_speed → playback speed on CapCut timeline (UI: "Tốc độ giọng đọc")
        self.sp_clip_speed = DoubleSpinBox()
        self.sp_clip_speed.setRange(0.1, 10.0)
        self.sp_clip_speed.setSingleStep(0.05)
        self.sp_clip_speed.setValue(default_clip)
        self.sp_clip_speed.setSuffix("x")
        self.sp_clip_speed.setMaximumWidth(120)
        self.sp_clip_speed.setToolTip(
            "Tốc độ phát giọng đọc trên timeline CapCut sau khi gắn. Ảnh hưởng thời lượng hiển thị."
        )

        self.cmb_tone = ComboBox()
        self.cmb_tone.addItem(
            "Giữ nguyên cao độ khi đổi tốc độ",
            userData=ToneModifyMode.PRESERVE_PITCH.value,
        )
        self.cmb_tone.addItem(
            "Đổi cao độ theo tốc độ",
            userData=ToneModifyMode.FOLLOW_SPEED.value,
        )
        self.cmb_tone.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Fixed)
        self.cmb_tone.setToolTip(
            "follow_speed bật is_tone_modify trong draft CapCut; preserve_pitch giữ pitch."
        )

        self.cmb_existing = ComboBox()
        self.cmb_existing.addItem(tr("replace_tts"), userData="replace_existing")
        self.cmb_existing.addItem(tr("skip_existing_tts"), userData="skip_existing")
        self.cmb_existing.setSizePolicy(QSizePolicy.Preferred, QSizePolicy.Fixed)

        # Row 0: language | voice + search
        grid.addWidget(self._field_label(tr("language_label")), 0, 0)
        grid.addWidget(self.cmb_lang, 0, 1)
        grid.addWidget(self._field_label(tr("voice_label")), 0, 2)
        grid.addWidget(voice_wrap, 0, 3)
        # Row 1: voice metadata under voice selector only
        grid.addWidget(self.lbl_voice_adv, 1, 3)
        # Row 2: clip speed | tone
        grid.addWidget(self._field_label(tr("voice_speed")), 2, 0)
        grid.addWidget(self.sp_clip_speed, 2, 1, Qt.AlignLeft)
        grid.addWidget(self._field_label(tr("pitch")), 2, 2)
        grid.addWidget(self.cmb_tone, 2, 3)
        # Row 3: existing TTS spans the field columns (no half-empty row)
        grid.addWidget(self._field_label(tr("existing_tts")), 3, 0)
        grid.addWidget(self.cmb_existing, 3, 1, 1, 3)
        return box

    def _build_advanced_panel(self) -> QWidget:
        wrap = QWidget()
        outer = QVBoxLayout(wrap)
        outer.setContentsMargins(0, 0, 0, 0)
        outer.setSpacing(4)

        toggle_row = QHBoxLayout()
        toggle_row.setContentsMargins(2, 0, 2, 0)
        self.btn_advanced = PushButton(FluentIcon.CHEVRON_RIGHT_MED, tr("advanced_options"))
        self.btn_advanced.setToolTip("Hiện/ẩn cache và căn chỉnh native")
        self.btn_advanced.setCursor(Qt.PointingHandCursor)
        toggle_row.addWidget(self.btn_advanced)
        toggle_row.addStretch(1)
        outer.addLayout(toggle_row)

        self.advanced_content = QWidget()
        adv = QGridLayout(self.advanced_content)
        adv.setContentsMargins(10, 6, 10, 8)
        adv.setHorizontalSpacing(10)
        adv.setVerticalSpacing(8)
        adv.setColumnMinimumWidth(0, 96)
        adv.setColumnMinimumWidth(2, 72)
        adv.setColumnStretch(1, 1)
        adv.setColumnStretch(3, 2)

        self.chk_cache = CheckBox(tr("use_cache"))
        self.chk_cache.setChecked(True)
        self.chk_align = CheckBox(tr("align_tts"))
        self.chk_align.setChecked(True)
        self.chk_align.setToolTip(
            "Trim/fade bằng source_timerange, không re-encode MP3."
        )

        self.sp_trim_frames = DoubleSpinBox()
        self.sp_trim_frames.setRange(0.0, 12.0)
        self.sp_trim_frames.setSingleStep(0.5)
        self.sp_trim_frames.setValue(3.0)
        self.sp_trim_frames.setSuffix(" khung hình")
        self.sp_trim_frames.setMaximumWidth(140)

        self.sp_fade_ms = DoubleSpinBox()
        self.sp_fade_ms.setRange(0.0, 20.0)
        self.sp_fade_ms.setSingleStep(1.0)
        self.sp_fade_ms.setValue(8.0)
        self.sp_fade_ms.setSuffix(" ms")
        self.sp_fade_ms.setMaximumWidth(120)

        self.lbl_align_hint = self._muted_label("3 khung hình @ 30 FPS = 100 ms")
        hf = self.lbl_align_hint.font()
        hf.setPointSize(max(8, hf.pointSize() - 1))
        self.lbl_align_hint.setFont(hf)

        adv.addWidget(self.chk_cache, 0, 0, 1, 2)
        adv.addWidget(self.chk_align, 0, 2, 1, 2)
        adv.addWidget(self._field_label(tr("trim_start")), 1, 0)
        adv.addWidget(self.sp_trim_frames, 1, 1, Qt.AlignLeft)
        adv.addWidget(self._field_label(tr("fade_in")), 1, 2)
        adv.addWidget(self.sp_fade_ms, 1, 3, Qt.AlignLeft)
        adv.addWidget(self.lbl_align_hint, 2, 1, 1, 3)

        self.advanced_content.setVisible(False)
        self.advanced_content.setMaximumHeight(0)  # collapse frees layout height
        outer.addWidget(self.advanced_content)
        return wrap

    def _build_workspace_split(self) -> QSplitter:
        split = QSplitter(Qt.Vertical)
        split.setChildrenCollapsible(False)
        split.addWidget(self._build_caption_workspace())
        split.addWidget(self._build_log_panel())
        self._split_stretch = (4, 1)
        split.setStretchFactor(0, self._split_stretch[0])
        split.setStretchFactor(1, self._split_stretch[1])
        split.setSizes([560, 160])
        return split

    def _build_caption_workspace(self) -> QWidget:
        cap_box = QWidget()
        cap_l = QVBoxLayout(cap_box)
        cap_l.setContentsMargins(0, 0, 0, 0)
        cap_l.setSpacing(6)

        tools = QHBoxLayout()
        tools.setSpacing(8)
        if _SearchLineEdit is not None:
            self.ed_search = _SearchLineEdit()
        else:
            self.ed_search = LineEdit()
        self.ed_search.setPlaceholderText(tr("search_caption"))
        self.ed_search.setClearButtonEnabled(True)
        self.ed_search.setMinimumWidth(220)

        self.btn_select_all = PushButton(tr("select_all"))
        self.btn_deselect_all = PushButton(tr("deselect_all"))
        self.chk_hide_empty = CheckBox(tr("hide_empty"))
        self.chk_hide_empty.setChecked(True)
        self.chk_only_no_tts = CheckBox(tr("without_tts"))
        self.chk_only_errors = CheckBox(tr("only_errors"))

        tools.addWidget(self.ed_search, 1)
        tools.addWidget(self.btn_select_all)
        tools.addWidget(self.btn_deselect_all)
        tools.addSpacing(4)
        tools.addWidget(self.chk_hide_empty)
        tools.addWidget(self.chk_only_no_tts)
        tools.addWidget(self.chk_only_errors)
        cap_l.addLayout(tools)

        self.lbl_selection = self._muted_label(tr("selection", selected=0, total=0, visible=0))
        sf = self.lbl_selection.font()
        sf.setPointSize(max(9, sf.pointSize() - 1))
        self.lbl_selection.setFont(sf)
        cap_l.addWidget(self.lbl_selection)

        self.table = QTableView()
        self.table.setModel(self._caption_proxy)
        header = self.table.horizontalHeader()
        header.setSectionResizeMode(0, QHeaderView.ResizeToContents)
        header.setSectionResizeMode(1, QHeaderView.ResizeToContents)
        header.setSectionResizeMode(2, QHeaderView.ResizeToContents)
        header.setSectionResizeMode(3, QHeaderView.ResizeToContents)
        header.setSectionResizeMode(4, QHeaderView.Stretch)
        header.setSectionResizeMode(5, QHeaderView.ResizeToContents)
        header.setSectionResizeMode(6, QHeaderView.ResizeToContents)
        header.setSectionResizeMode(7, QHeaderView.ResizeToContents)
        header.setSectionResizeMode(8, QHeaderView.Interactive)
        self.table.setColumnWidth(8, 140)
        self.table.setSelectionBehavior(QAbstractItemView.SelectRows)
        self.table.setSelectionMode(QAbstractItemView.ExtendedSelection)
        self.table.setEditTriggers(QAbstractItemView.NoEditTriggers)
        self.table.verticalHeader().setVisible(False)
        self.table.verticalHeader().setDefaultSectionSize(28)
        self.table.setAlternatingRowColors(True)
        # Fixed row height ≈ uniform rows (QTableView has no setUniformRowHeights).
        self.table.verticalHeader().setSectionResizeMode(QHeaderView.Fixed)
        self.table.setWordWrap(False)
        self.table.setShowGrid(False)
        self.table.setMinimumHeight(220)
        self.table.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Expanding)
        cap_l.addWidget(self.table, 1)

        self.lbl_table_empty = self._muted_label("Chưa có caption — chọn project để bắt đầu.")
        self.lbl_table_empty.setAlignment(Qt.AlignCenter)
        self.lbl_table_empty.setMinimumHeight(40)
        cap_l.addWidget(self.lbl_table_empty)
        return cap_box

    def _build_log_panel(self) -> QWidget:
        log_box = QGroupBox("Nhật ký")
        log_l = QVBoxLayout(log_box)
        log_l.setContentsMargins(8, 6, 8, 6)
        log_l.setSpacing(4)

        log_tools = QHBoxLayout()
        self.btn_log_clear = ToolButton(FluentIcon.DELETE)
        self.btn_log_clear.setToolTip(tr("clear_log"))
        self.btn_log_copy = ToolButton(FluentIcon.COPY)
        self.btn_log_copy.setToolTip(tr("copy_log"))
        self.btn_log_save = ToolButton(FluentIcon.SAVE)
        self.btn_log_save.setToolTip(tr("save_log"))
        log_tools.addStretch(1)
        log_tools.addWidget(self.btn_log_clear)
        log_tools.addWidget(self.btn_log_copy)
        log_tools.addWidget(self.btn_log_save)
        log_l.addLayout(log_tools)

        self.log = TextEdit()
        self.log.setReadOnly(True)
        self.log.setFont(QFont("Consolas", 10))
        self.log.document().setMaximumBlockCount(5000)
        self.log.setMinimumHeight(80)
        log_l.addWidget(self.log)
        log_box.setMinimumHeight(110)
        return log_box

    def _build_footer(self) -> QHBoxLayout:
        foot = QHBoxLayout()
        foot.setSpacing(8)
        self.progress = ProgressBar()
        self.progress.setRange(0, 1000)
        self.progress.setValue(0)
        self.lbl_progress = QLabel(tr("ready"))
        self.lbl_progress.setMinimumWidth(140)
        self.lbl_progress.setAlignment(Qt.AlignVCenter | Qt.AlignLeft)
        self.btn_cancel = PushButton(tr("cancel"))
        self.btn_cancel.setEnabled(False)
        _gen_icon = getattr(FluentIcon, "PLAY", None) or getattr(FluentIcon, "ACCEPT", None)
        self.btn_generate = (
            PrimaryPushButton(_gen_icon, tr("generate_attach"))
            if _gen_icon is not None
            else PrimaryPushButton(tr("generate_attach"))
        )
        self.btn_generate.setEnabled(False)
        self.btn_settings = ToolButton(FluentIcon.SETTING)
        self.btn_settings.setToolTip(tr("app_settings"))
        foot.addWidget(self.btn_settings)
        foot.addWidget(self.progress, 1)
        foot.addWidget(self.lbl_progress)
        foot.addWidget(self.btn_cancel)
        foot.addWidget(self.btn_generate)
        return foot

    def _connect_ui_signals(self):
        self.btn_browse.clicked.connect(self._browse_project)
        self.btn_reload.clicked.connect(self._reload_project)
        self.btn_export_srt.clicked.connect(self._export_srt)
        self.btn_split.clicked.connect(self._split_project)
        self.btn_merge.clicked.connect(self._merge_videos)
        self.cmb_lang.currentIndexChanged.connect(self._refresh_voice_combo)
        self.ed_voice_search.textChanged.connect(self._refresh_voice_combo)
        self.cmb_voice.currentIndexChanged.connect(self._on_voice_changed)
        self.btn_advanced.clicked.connect(self._toggle_advanced)
        self.chk_align.stateChanged.connect(self._on_align_toggled)
        self.sp_trim_frames.valueChanged.connect(self._update_align_hint)
        self.ed_search.textChanged.connect(self._schedule_filter)
        self.btn_select_all.clicked.connect(self._on_select_all)
        self.btn_deselect_all.clicked.connect(self._on_deselect_all)
        self.chk_hide_empty.stateChanged.connect(self._schedule_filter)
        self.chk_only_no_tts.stateChanged.connect(self._schedule_filter)
        self.chk_only_errors.stateChanged.connect(self._schedule_filter)
        self._caption_model.dataChanged.connect(self._on_table_data_changed)
        self.btn_log_clear.clicked.connect(self._clear_log)
        self.btn_log_copy.clicked.connect(self._copy_log)
        self.btn_log_save.clicked.connect(self._save_log)
        self.btn_cancel.clicked.connect(self._cancel)
        self.btn_generate.clicked.connect(self._start_generate)
        self.btn_settings.clicked.connect(self._open_settings)
        # initial alignment enable state
        self._on_align_toggled(self.chk_align.checkState())

    def _open_settings(self):
        dialog = SettingsDialog(self)
        dialog.settings_saved.connect(self._on_settings_saved)
        dialog.exec()

    def _on_settings_saved(self):
        self._apply_theme()
        self._load_voices()

    # ------------------------------------------------------------------
    # UI state
    # ------------------------------------------------------------------
    def _apply_ui_state(self, state: UiState):
        self._ui_state = state
        busy = state in {UiState.GENERATING, UiState.CANCELLING}
        has_project = state != UiState.IDLE_NO_PROJECT or bool(self.ed_project.text().strip())

        if state == UiState.IDLE_NO_PROJECT:
            self.btn_generate.setEnabled(False)
            self.btn_generate.setText(tr("generate_attach"))
            self.btn_cancel.setEnabled(False)
            self.btn_cancel.setText(tr("cancel"))
            self.btn_reload.setEnabled(bool(self.ed_project.text().strip()))
            self.btn_export_srt.setEnabled(False)
            self.btn_split.setEnabled(False)
            self.btn_merge.setEnabled(True)  # merge only needs exported videos
            self.btn_browse.setEnabled(True)
            self.lbl_progress.setText(tr("ready"))
        elif state == UiState.IDLE_READY:
            self.btn_generate.setEnabled(True)
            self.btn_generate.setText(tr("generate_attach"))
            self.btn_cancel.setEnabled(False)
            self.btn_cancel.setText(tr("cancel"))
            self.btn_reload.setEnabled(True)
            self.btn_export_srt.setEnabled(True)
            self.btn_split.setEnabled(True)
            self.btn_merge.setEnabled(True)
            self.btn_browse.setEnabled(True)
        elif state == UiState.GENERATING:
            self.btn_generate.setEnabled(False)
            self.btn_generate.setText(tr("generating"))
            self.btn_cancel.setEnabled(True)
            self.btn_cancel.setText(tr("cancel"))
            self.btn_reload.setEnabled(False)
            self.btn_export_srt.setEnabled(False)
            self.btn_split.setEnabled(False)
            self.btn_merge.setEnabled(False)
            self.btn_browse.setEnabled(False)
        elif state == UiState.CANCELLING:
            self.btn_generate.setEnabled(False)
            self.btn_generate.setText(tr("cancelling"))
            self.btn_cancel.setEnabled(False)
            self.btn_cancel.setText(tr("cancel"))
            self.btn_reload.setEnabled(False)
            self.btn_export_srt.setEnabled(False)
            self.btn_split.setEnabled(False)
            self.btn_merge.setEnabled(False)
            self.btn_browse.setEnabled(False)

        # Settings + caption selection only when idle (table still scrollable)
        for w in (
            self.cmb_lang,
            self.cmb_voice,
            self.ed_voice_search,
            self.sp_clip_speed,
            self.cmb_tone,
            self.cmb_existing,
            self.chk_cache,
            self.chk_align,
            self.btn_advanced,
            self.ed_search,
            self.btn_select_all,
            self.btn_deselect_all,
            self.chk_hide_empty,
            self.chk_only_no_tts,
            self.chk_only_errors,
        ):
            w.setEnabled(not busy)
        # Freeze checkboxes during run to avoid selection drift vs snapshot
        self._caption_model.set_checks_enabled(not busy)
        if busy:
            self.sp_trim_frames.setEnabled(False)
            self.sp_fade_ms.setEnabled(False)
            self.lbl_align_hint.setEnabled(False)
        else:
            self._on_align_toggled(self.chk_align.checkState())

        _ = has_project

    def _toggle_advanced(self):
        self._advanced_open = not self._advanced_open
        self.advanced_content.setVisible(self._advanced_open)
        # Ensure collapsed panel reports zero height to layout
        if self._advanced_open:
            self.advanced_content.setMaximumHeight(16777215)
        else:
            self.advanced_content.setMaximumHeight(0)
        icon = FluentIcon.CHEVRON_DOWN_MED if self._advanced_open else FluentIcon.CHEVRON_RIGHT_MED
        self.btn_advanced.setIcon(icon)
        self.advanced_content.updateGeometry()
        if self.centralWidget() is not None:
            self.centralWidget().updateGeometry()

    def _on_align_toggled(self, state):
        # Qt6 may pass CheckState enum; avoid int(CheckState) which raises TypeError
        if isinstance(state, bool):
            enabled = state
        else:
            enabled = state == Qt.Checked
        if self._ui_state in {UiState.GENERATING, UiState.CANCELLING}:
            enabled = False
        self.sp_trim_frames.setEnabled(enabled)
        self.sp_fade_ms.setEnabled(enabled)
        self.lbl_align_hint.setEnabled(enabled)

    def _on_select_all(self):
        self._select_all(True)

    def _on_deselect_all(self):
        self._select_all(False)

    def _on_table_data_changed(self, *_args):
        self._update_selection_label()

    def _clear_log(self):
        self.log.clear()

    def _update_empty_state(self):
        total = self._caption_model.rowCount()
        visible = self._caption_proxy.rowCount()
        if total == 0:
            self.lbl_table_empty.setText("Chưa có caption — chọn project để bắt đầu.")
            self.lbl_table_empty.setVisible(True)
        elif visible == 0:
            self.lbl_table_empty.setText(tr("empty_filter"))
            self.lbl_table_empty.setVisible(True)
        else:
            self.lbl_table_empty.setVisible(False)

    # ------------------------------------------------------------------
    # Voices
    # ------------------------------------------------------------------
    def _load_voices(self):
        try:
            cat = self.service.get_voice_catalog(reload=True)
            self._voices = cat.voices
        except Exception as e:
            self._append_log("ERROR", f"Không load được danh sách giọng: {e}")
            self._voices = []
            return
        self.cmb_lang.blockSignals(True)
        self.cmb_lang.clear()
        self.cmb_lang.addItem(tr("all"), userData="")
        for lan, locale in cat.languages():
            label = locale or lan or "?"
            self.cmb_lang.addItem(label, userData=lan or locale)
        self.cmb_lang.blockSignals(False)
        self._refresh_voice_combo()

    def _refresh_voice_combo(self):
        lan = self.cmb_lang.currentData() or ""
        q = self.ed_voice_search.text().strip().lower()
        self.cmb_voice.blockSignals(True)
        self.cmb_voice.clear()
        for v in self._voices:
            if lan and v.language_code != lan and v.locale != lan and not v.locale.startswith(str(lan)):
                continue
            if q:
                hay = f"{v.display_name} {v.voice_type} {v.resource_id}".lower()
                if q not in hay:
                    continue
            self.cmb_voice.addItem(v.display_name, userData=v)
        self.cmb_voice.blockSignals(False)
        self._on_voice_changed()

    def _on_voice_changed(self):
        v = self.cmb_voice.currentData()
        if not v:
            self.lbl_voice_adv.setText("")
            return
        self.lbl_voice_adv.setText(f"{v.voice_type} · resource_id={v.resource_id}")
        self.lbl_voice_adv.setToolTip(self.lbl_voice_adv.text())

    def _update_align_hint(self):
        frames = float(self.sp_trim_frames.value())
        fps = 30.0
        if self.service._info is not None and self.service._info.fps > 0:
            fps = float(self.service._info.fps)
        ms = int(round(frames / fps * 1000.0)) if fps else 0
        self.lbl_align_hint.setText(f"{frames:g} khung hình @ {fps:g} FPS = {ms} ms")

    # ------------------------------------------------------------------
    # Project
    # ------------------------------------------------------------------
    def _default_project_dir(self) -> str:
        """Start folder for CapCut project pickers (AutoVideo outputs / last project)."""
        cur = self.ed_project.text().strip()
        if cur:
            p = Path(cur)
            if p.is_file():
                return str(p.parent)
            if p.is_dir():
                return str(p)
        cfg = self.config.capcut_projects_path
        if cfg is not None:
            return str(cfg)
        return ""

    def _browse_project(self):
        start = self._default_project_dir()
        path, _ = QFileDialog.getOpenFileName(
            self,
            "Chọn draft_content.json (hoặc Hủy để chọn thư mục)",
            start,
            "CapCut Draft (draft_content.json);;JSON (*.json);;All (*.*)",
        )
        if not path:
            dir_path = QFileDialog.getExistingDirectory(
                self, "Chọn thư mục project CapCut", start
            )
            if not dir_path:
                return
            path = dir_path
        self.ed_project.setText(path)
        self.ed_project.setToolTip(path)
        self._load_project(path)

    def _reload_project(self):
        path = self.ed_project.text().strip()
        if path:
            self._load_project(path)

    def _export_srt(self):
        info = self.service._info
        if info is None:
            self._notify("warning", "Chưa có project", "Hãy chọn project CapCut trước.")
            return

        default_path = info.project_directory / f"{info.project_directory.name}.srt"
        output_path, _ = QFileDialog.getSaveFileName(
            self,
            "Xuất caption thành SRT",
            str(default_path),
            "SubRip Subtitle (*.srt)",
        )
        if not output_path:
            return
        if Path(output_path).suffix.lower() != ".srt":
            output_path += ".srt"

        try:
            count = self.service.export_srt(output_path)
        except Exception as e:
            self._notify("error", "Không xuất được SRT", str(e), duration=5000)
            return

        self._append_log("SUCCESS", f"Đã xuất {count} caption: {output_path}", stage="SRT")
        self._notify("success", "Đã xuất SRT", f"{count} caption · {output_path}", duration=4000)

    def _split_project(self):
        info = self.service._info
        if info is None or self.service.reader.draft is None:
            self._notify("warning", "Chưa có project", "Hãy chọn project CapCut trước.")
            return
        if self.worker and self.worker.isRunning():
            return
        try:
            source_draft = self.service.reader.get_draft_copy()
        except Exception as e:
            self._notify("error", "Không đọc draft", str(e))
            return

        total_us = int(source_draft.get("duration") or info.duration_us or 0)
        if total_us < 2_000_000:
            self._notify("warning", "Project quá ngắn", "Cần ít nhất ~2s để tách 2 phần.")
            return

        part1 = info.project_directory.parent / f"{info.project_directory.name}_part1"
        part2 = info.project_directory.parent / f"{info.project_directory.name}_part2"
        if part1.exists() or part2.exists():
            box = QMessageBox(self)
            box.setIcon(QMessageBox.Warning)
            box.setWindowTitle(tr("split_project"))
            box.setText(
                f"Thư mục đích đã tồn tại:\n{part1.name} / {part2.name}\n\n"
                "Xóa chúng rồi chạy lại, hoặc đổi tên project nguồn."
            )
            box.addButton(tr("cancel"), QMessageBox.RejectRole)
            box.exec()
            return

        edges = [c.start_us for c in self._captions if c.start_us > 0]
        box = QMessageBox(self)
        box.setIcon(QMessageBox.Question)
        box.setWindowTitle(tr("split_project"))
        box.setText(
            "Tách project thành 2 bản copy (part1 / part2) theo mốc giữa timeline.\n"
            "Project gốc không bị sửa.\n\n"
            f"Nguồn: {info.project_directory.name}\n"
            f"Thời lượng: {info.duration_display}\n"
            f"Xuất ra:\n  {part1}\n  {part2}\n\n"
            "Sau đó mở từng part trong CapCut → Export → dùng Ghép video."
        )
        ok = box.addButton("Tách", QMessageBox.AcceptRole)
        box.addButton(tr("cancel"), QMessageBox.RejectRole)
        box.exec()
        if box.clickedButton() is not ok:
            return

        self.progress.setValue(0)
        self.lbl_progress.setText(tr("split_project"))
        self._apply_ui_state(UiState.GENERATING)

        self.worker = SplitWorker(
            source_project_dir=info.project_directory,
            source_draft=source_draft,
            source_draft_path=info.draft_path,
            caption_edges_us=edges,
        )
        self.worker.progress.connect(self._on_progress)
        self.worker.finished_ok.connect(self._on_split_finished)
        self.worker.failed.connect(self._on_failed)
        self.worker.start()

    def _on_split_finished(self, result):
        if self._closing:
            return
        self.worker = None
        self.progress.setValue(1000)
        self.lbl_progress.setText(tr("ready"))
        self._apply_ui_state(UiState.IDLE_READY)
        cut_s = result.cut_us / 1_000_000.0
        self._append_log(
            "SUCCESS",
            f"Đã tách tại {cut_s:.2f}s → {result.part1_dir.name} ({result.part1_segments} seg) + "
            f"{result.part2_dir.name} ({result.part2_segments} seg)",
            stage="Split",
        )
        self._append_log("INFO", f"Part1: {result.part1_dir}", stage="Split")
        self._append_log("INFO", f"Part2: {result.part2_dir}", stage="Split")
        self._notify(
            "success",
            "Đã tách project",
            f"{result.part1_dir.name} + {result.part2_dir.name}",
            duration=6000,
        )

    def _merge_videos(self):
        if self.worker and self.worker.isRunning():
            return
        paths, _ = QFileDialog.getOpenFileNames(
            self,
            tr("merge_videos"),
            "",
            "Video (*.mp4 *.mov *.mkv *.m4v);;All (*.*)",
        )
        if not paths:
            return
        if len(paths) < 2:
            self._notify("warning", tr("merge_videos"), "Chọn ít nhất 2 video (theo thứ tự part1 → part2).")
            return

        default_out = str(Path(paths[0]).with_name(Path(paths[0]).stem + "_merged.mp4"))
        out_path, _ = QFileDialog.getSaveFileName(
            self,
            "Lưu video đã ghép",
            default_out,
            "MP4 (*.mp4);;All (*.*)",
        )
        if not out_path:
            return
        if Path(out_path).suffix.lower() not in {".mp4", ".mov", ".mkv", ".m4v"}:
            out_path += ".mp4"

        self.progress.setValue(0)
        self.lbl_progress.setText(tr("merge_videos"))
        self._apply_ui_state(UiState.GENERATING)

        ff = "ffmpeg"
        try:
            ff = AppConfig().ffmpeg_path
        except Exception:
            pass

        self.worker = MergeWorker(inputs=paths, output=out_path, ffmpeg_path=ff)
        self.worker.progress.connect(self._on_progress)
        self.worker.finished_ok.connect(self._on_merge_finished)
        self.worker.failed.connect(self._on_failed)
        self.worker.start()

    def _on_merge_finished(self, result):
        if self._closing:
            return
        self.worker = None
        self.progress.setValue(1000)
        self.lbl_progress.setText(tr("ready"))
        # Restore ready/no-project based on whether a project is loaded
        if self.service._info is not None:
            self._apply_ui_state(UiState.IDLE_READY)
        else:
            self._apply_ui_state(UiState.IDLE_NO_PROJECT)
        mode = "re-encode" if result.used_reencode else "stream-copy"
        self._append_log(
            "SUCCESS",
            f"Đã ghép video ({mode}): {result.output_path}",
            stage="Merge",
        )
        self._notify("success", "Đã ghép video", f"{mode} · {result.output_path}", duration=6000)

    def _set_project_info(self, info) -> None:
        """Theme-safe plain-text summary (no hardcoded HTML colors)."""
        path_str = str(info.draft_path)
        line1 = (
            f"{info.project_name} · {info.resolution} · {info.fps:g} FPS · "
            f"{info.duration_display} · caption={info.caption_count} · "
            f"đã có TTS={info.caption_with_tts_count} · rỗng={info.empty_caption_count}"
        )
        self.lbl_info.setText(f"{line1}\n{path_str}")
        self.lbl_info.setToolTip(path_str)

    def _load_project(self, path: str):
        try:
            info = self.service.load_project(path)
            self._captions = self.service.get_captions()
            self.service.inspect_project()
        except Exception as e:
            self._notify("error", "Không đọc được project", str(e), duration=5000)
            self._apply_ui_state(UiState.IDLE_NO_PROJECT)
            return

        self._set_project_info(info)
        self._update_align_hint()
        looks_like_capcut = (
            (info.project_directory / "draft_meta_info.json").exists()
            or (info.project_directory / "Timelines").is_dir()
        )
        if not looks_like_capcut:
            self._append_log(
                "WARNING",
                "Folder này không giống project CapCut đầy đủ (thiếu draft_meta_info.json / Timelines). "
                "Hãy chọn thư mục trong AppData\\Local\\CapCut\\User Data\\Projects\\com.lveditor.draft\\… "
                "nếu không CapCut sẽ không thấy audio.",
                stage="Project",
            )
        self._populate_table()
        self._apply_ui_state(UiState.IDLE_READY)
        for line in [
            f"Project: {info.project_name}",
            f"ID: {info.project_id}",
            f"Version: {info.version} / {info.new_version}",
            f"Canvas: {info.width}×{info.height} @ {info.fps:g} FPS",
            f"Duration: {info.duration_display}",
            f"Tracks: video={info.video_track_count}, audio={info.audio_track_count}, text={info.text_track_count}",
            f"Captions: total={info.caption_count}, existing_tts={info.caption_with_tts_count}, empty={info.empty_caption_count}",
        ]:
            self._append_log("INFO", line, stage="Project")

    def _populate_table(self):
        self._caption_model.set_captions(self._captions)
        self._filter_table()

    def _schedule_filter(self, *_args):
        self._filter_timer.start()

    def _filter_table(self):
        self._caption_proxy.set_filters(
            query=self.ed_search.text(),
            hide_empty=self.chk_hide_empty.isChecked(),
            only_no_tts=self.chk_only_no_tts.isChecked(),
            only_errors=self.chk_only_errors.isChecked(),
        )
        self._update_selection_label()
        self._update_empty_state()

    def _update_selection_label(self):
        selected, total, _ = self._caption_model.selection_counts()
        visible = self._caption_proxy.rowCount()
        self.lbl_selection.setText(
            tr("selection", selected=selected, total=total, visible=visible)
        )

    def _select_all(self, on: bool):
        # Only touch currently visible (filtered) rows — matches old behavior.
        rows = sorted(self._caption_proxy.visible_source_rows())
        self._caption_model.set_all_checked(on, only_rows=rows)
        self._update_selection_label()

    # ------------------------------------------------------------------
    # Generate
    # ------------------------------------------------------------------
    def _start_generate(self):
        if self.worker and self.worker.isRunning():
            return
        if self.service.is_capcut_running():
            box = QMessageBox(self)
            box.setIcon(QMessageBox.Warning)
            box.setWindowTitle("CapCut đang chạy")
            box.setText(
                "CapCut đang chạy.\n\n"
                "Hãy đóng project CapCut trước khi tiếp tục để tránh project bị ghi đè."
            )
            retry = box.addButton("Kiểm tra lại", QMessageBox.AcceptRole)
            box.addButton("Hủy", QMessageBox.RejectRole)
            box.exec()
            if box.clickedButton() is retry:
                if self.service.is_capcut_running():
                    self._notify("warning", "CapCut vẫn đang chạy", "Đóng CapCut rồi thử lại.")
                    return
            else:
                return

        selected = self._caption_model.selected_segment_ids()
        if not selected:
            self._notify("warning", "Chưa chọn caption", "Chọn ít nhất một caption để tạo TTS.")
            return

        voice = self.cmb_voice.currentData()
        if not voice:
            self._notify("warning", "Chưa chọn giọng đọc", "Chọn một giọng trong danh sách.")
            return

        tone_raw = self.cmb_tone.currentData()
        tone = ToneModifyMode(tone_raw) if not isinstance(tone_raw, ToneModifyMode) else tone_raw
        existing = self.cmb_existing.currentData()

        self.progress.setValue(0)
        self.lbl_progress.setText("0%")
        self._apply_ui_state(UiState.GENERATING)

        alignment = NativeAudioAlignmentSettings(
            enabled=self.chk_align.isChecked(),
            leading_trim_frames=float(self.sp_trim_frames.value()),
            fade_in_ms=float(self.sp_fade_ms.value()),
        )
        self.worker = GenerateWorker(
            service=self.service,
            selected_ids=selected,
            voice_type=voice.voice_type,
            resource_id=voice.resource_id,
            voice_display_name=voice.display_name,
            tts_rate=1.0,  # API rate fixed; UI only exposes timeline clip speed
            clip_speed=float(self.sp_clip_speed.value()),
            tone_mode=tone,
            existing_mode=str(existing),
            use_cache=self.chk_cache.isChecked(),
            alignment=alignment,
        )
        self.worker.log_event.connect(self._on_log_event)
        self.worker.progress.connect(self._on_progress)
        self.worker.item_done.connect(self._on_item)
        self.worker.finished_ok.connect(self._on_finished)
        self.worker.failed.connect(self._on_failed)
        self.worker.start()

    def _cancel(self):
        if self.worker and self.worker.isRunning():
            self.worker.cancel()
            self._apply_ui_state(UiState.CANCELLING)
            self._append_log("WARNING", "Đang hủy…", stage="cancelled")

    def closeEvent(self, event):
        """Cancel running worker and wait so QThread is not destroyed mid-run."""
        worker = self.worker
        if worker is not None and worker.isRunning():
            self._closing = True
            worker.cancel()
            self._apply_ui_state(UiState.CANCELLING)
            self._append_log("WARNING", "Đóng ứng dụng — đang hủy tác vụ…", stage="cancelled")
            if not worker.wait(8000):
                self._append_log(
                    "ERROR",
                    "Worker vẫn chạy sau 8s — không đóng cửa sổ để tránh crash QThread.",
                    stage="cancelled",
                )
                event.ignore()
                return
            self.worker = None
        event.accept()

    def _on_log_event(self, ev: GenerationLogEvent):
        if self._closing:
            return
        self._append_log(ev.level, ev.message, stage=ev.stage)

    def _on_progress(self, p: float, msg: str):
        if self._closing:
            return
        if self.worker and getattr(self.worker, "_cancel", False):
            p = min(p, 0.99)
            if self._ui_state != UiState.CANCELLING:
                self._apply_ui_state(UiState.CANCELLING)
        value = int(p * 1000)
        label = f"{p * 100:.0f}% {msg}".strip()
        # Skip no-op bar repaints when consecutive signals map to the same tick.
        if value == self.progress.value() and label == self.lbl_progress.text():
            return
        self.progress.setValue(value)
        self.lbl_progress.setText(label)

    def _on_item(self, idx: int, res: dict):
        if self._closing:
            return
        # Coalesce rapid worker signals; flush on a short timer.
        self._pending_item_updates[int(idx)] = res
        if not self._item_flush_timer.isActive():
            self._item_flush_timer.start()

    def _flush_item_updates(self):
        if self._closing or not self._pending_item_updates:
            self._pending_item_updates.clear()
            return
        pending = self._pending_item_updates
        self._pending_item_updates = {}
        self._caption_model.apply_item_results(pending)
        if self.chk_only_errors.isChecked():
            self._filter_table()

    def _on_finished(self, result: GenerationResult):
        if self._closing:
            self.worker = None
            return
        self._item_flush_timer.stop()
        self._flush_item_updates()
        cancelled = bool((result.extra or {}).get("cancelled")) or any(
            "cancel" in (e or "").lower() for e in (result.errors or [])
        )
        ready = UiState.IDLE_READY if self.ed_project.text().strip() else UiState.IDLE_NO_PROJECT
        self._apply_ui_state(ready)
        if cancelled:
            self.progress.setValue(min(self.progress.value(), 990))
            self.lbl_progress.setText("Đã hủy")
            self.btn_generate.setText("Tạo và gắn TTS")
            self._notify(
                "warning",
                "Đã hủy",
                f"Đã tạo {result.generated}, cache {result.cached}, lỗi {result.failed}, bỏ qua {result.skipped}.",
                duration=5000,
            )
            return
        self.progress.setValue(1000)
        self.lbl_progress.setText("Hoàn tất")
        if result.success:
            title = "Hoàn thành với cảnh báo" if result.completed_with_warnings else "Thành công"
            self._notify(
                "warning" if result.completed_with_warnings else "success",
                title,
                f"Gắn {result.attached}/{result.selected}, lỗi {result.failed}, "
                f"tạo mới {result.generated}, cache {result.cached}, bỏ qua {result.skipped}. "
                f"Chi tiết trong nhật ký.",
                duration=6500,
            )
            self.btn_generate.setText("Tạo lại")
            self._reload_project()
        else:
            self._notify(
                "error",
                "Thất bại",
                "\n".join(result.errors[:3])
                or f"generated={result.generated} failed={result.failed}",
                duration=8000,
            )

    def _on_failed(self, msg: str):
        if self._closing:
            self.worker = None
            return
        self.worker = None
        self._item_flush_timer.stop()
        self._flush_item_updates()
        ready = UiState.IDLE_READY if self.ed_project.text().strip() else UiState.IDLE_NO_PROJECT
        self._apply_ui_state(ready)
        self.btn_generate.setText(tr("generate_attach"))
        self.lbl_progress.setText("Lỗi")
        self._notify("error", "Lỗi", msg, duration=8000)

    # ------------------------------------------------------------------
    # Log helpers
    # ------------------------------------------------------------------
    def _notify(self, level: str, title: str, content: str = "", *, duration: int = 3000):
        notify = {
            "success": InfoBar.success,
            "warning": InfoBar.warning,
            "error": InfoBar.error,
            "info": InfoBar.info,
        }.get(level, InfoBar.info)
        notify(
            title=title,
            content=content,
            orient=Qt.Horizontal,
            isClosable=True,
            position=InfoBarPosition.TOP_RIGHT,
            duration=duration,
            parent=self,
        )

    def _append_log(self, level: str, message: str, stage: str | None = None):
        from datetime import datetime

        ts = datetime.now().strftime("%H:%M:%S")
        prefix = f"[{stage}] " if stage else ""
        line = f"{ts} {level:<7} {prefix}{message}"
        # Prefer palette-based colors via CSS vars-like fixed accents only for severity
        # (severity is dark/light safe enough for log accents; body uses default text color)
        color = {
            "ERROR": "#EF4444",
            "WARNING": "#D97706",
            "SUCCESS": "#059669",
            "CACHED": "#0D9488",
            "INFO": "",
            "DEBUG": "",
        }.get(level.upper(), "")
        if color:
            self.log.append(f'<span style="color:{color}">{line}</span>')
        else:
            # Let QTextEdit use theme foreground for info/debug
            self.log.append(line)
        self.log.moveCursor(QTextCursor.End)

    def _copy_log(self):
        QApplication.clipboard().setText(self.log.toPlainText())

    def _save_log(self):
        path, _ = QFileDialog.getSaveFileName(
            self, "Lưu nhật ký", "generation.log", "Log (*.log)"
        )
        if not path:
            return
        Path(path).write_text(self.log.toPlainText(), encoding="utf-8")
