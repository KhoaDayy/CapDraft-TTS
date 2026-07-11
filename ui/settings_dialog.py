"""Persistent application settings dialog."""

from __future__ import annotations

from pathlib import Path

from PySide6.QtCore import Qt, Signal
from PySide6.QtWidgets import (
    QApplication,
    QFileDialog,
    QFormLayout,
    QHBoxLayout,
    QLabel,
    QMessageBox,
    QTabWidget,
)
from qfluentwidgets import (
    ComboBox,
    Dialog,
    DoubleSpinBox,
    FluentIcon,
    LineEdit,
    PushButton,
    SpinBox,
)

from core.capcut_project.voice_catalog import DEFAULT_VOICE_CATALOG_URL
from core.capcut_project.voice_catalog_updater import (
    VoiceCatalogUpdateError,
    update_voice_catalog_from_url,
)
from core.config import AppConfig
from core.i18n import SUPPORTED_LANGUAGES, set_language, tr


class SettingsDialog(Dialog):
    """Edit settings without exposing the raw JSON file."""

    settings_saved = Signal()

    def __init__(self, parent=None):
        cfg = AppConfig()
        set_language(cfg.language)
        super().__init__(
            tr("settings"),
            tr("settings_description"),
            parent,
        )
        self.cfg = cfg
        self._build_ui()
        self.yesButton.setText(tr("save_settings"))
        self.cancelButton.setText(tr("cancel"))
        self.yesButton.clicked.disconnect()
        self.yesButton.clicked.connect(self._save)
        self.setMinimumWidth(680)

    def _build_ui(self):
        tabs = QTabWidget()
        tabs.addTab(self._appearance_tab(), tr("appearance"))
        tabs.addTab(self._paths_tab(), tr("paths"))
        tabs.addTab(self._voices_tab(), tr("voices"))
        tabs.addTab(self._performance_tab(), tr("performance"))
        self.textLayout.addWidget(tabs)

    def _appearance_tab(self):
        page, form = self._form_page()
        self.language_combo = ComboBox()
        for code, label in SUPPORTED_LANGUAGES.items():
            self.language_combo.addItem(label, userData=code)
        self.language_combo.setCurrentIndex(max(0, self.language_combo.findData(self.cfg.language)))

        self.theme_combo = ComboBox()
        for mode, key in (("auto", "theme_auto"), ("light", "theme_light"), ("dark", "theme_dark")):
            self.theme_combo.addItem(tr(key), userData=mode)
        self.theme_combo.setCurrentIndex(max(0, self.theme_combo.findData(self.cfg.theme_mode)))

        note = QLabel(tr("restart_language"))
        note.setWordWrap(True)
        form.addRow(tr("language"), self.language_combo)
        form.addRow(tr("theme"), self.theme_combo)
        form.addRow("", note)
        return page

    def _paths_tab(self):
        page, form = self._form_page()
        self.capcut_tts_path = self._path_row("capcut_tts_path", directory=True)
        self.device_json_path = self._path_row("device_json_path", file_filter="JSON (*.json)")
        self.ffprobe_path = LineEdit()
        self.ffprobe_path.setText(str(self.cfg.get("ffprobe_path", "ffprobe")))
        form.addRow("CapCut TTS API", self.capcut_tts_path.parentWidget())
        form.addRow("Thiết bị (device.json)", self.device_json_path.parentWidget())
        form.addRow("FFprobe (optional)", self.ffprobe_path)
        return page

    def _voices_tab(self):
        page, form = self._form_page()
        self.voice_catalog_url = LineEdit()
        self.voice_catalog_url.setText(self.cfg.voice_catalog_url)
        self.voice_catalog_url.setPlaceholderText(DEFAULT_VOICE_CATALOG_URL)
        self.btn_update_voices = PushButton(FluentIcon.SYNC, tr("reload_list"))
        self.btn_update_voices.setToolTip("Tải danh sách giọng từ URL (không lưu file local)")
        self.btn_update_voices.clicked.connect(self._update_voice_catalog)
        self.lbl_voice_update_status = QLabel("")
        self.lbl_voice_update_status.setTextInteractionFlags(Qt.TextSelectableByMouse)
        form.addRow(tr("voice_catalog_url"), self.voice_catalog_url)
        form.addRow("", self.btn_update_voices)
        form.addRow("", self.lbl_voice_update_status)
        return page

    def _performance_tab(self):
        page, form = self._form_page()
        self.chunk_size = SpinBox()
        self.chunk_size.setRange(1, 100)
        self.chunk_size.setValue(int(self.cfg.get("tts_chunk_size", 25)))
        self.parallel_chunks = SpinBox()
        self.parallel_chunks.setRange(1, 16)
        self.parallel_chunks.setValue(int(self.cfg.get("tts_parallel_chunks", 4)))
        self.download_workers = SpinBox()
        self.download_workers.setRange(1, 32)
        self.download_workers.setValue(int(self.cfg.get("tts_download_workers", 8)))
        self.poll_interval = DoubleSpinBox()
        self.poll_interval.setRange(0.1, 10.0)
        self.poll_interval.setSingleStep(0.1)
        self.poll_interval.setSuffix(tr("seconds_suffix"))
        self.poll_interval.setValue(float(self.cfg.get("tts_poll_interval_sec", 1.0)))
        self.max_backups = SpinBox()
        self.max_backups.setRange(1, 100)
        self.max_backups.setValue(int(self.cfg.get("max_backups", 10)))
        form.addRow(tr("captions_per_batch"), self.chunk_size)
        form.addRow(tr("parallel_batches"), self.parallel_chunks)
        form.addRow(tr("audio_workers"), self.download_workers)
        form.addRow(tr("poll_interval"), self.poll_interval)
        form.addRow(tr("backup_count"), self.max_backups)
        return page

    @staticmethod
    def _form_page():
        from PySide6.QtWidgets import QWidget

        page = QWidget()
        form = QFormLayout(page)
        form.setContentsMargins(18, 18, 18, 18)
        form.setHorizontalSpacing(18)
        form.setVerticalSpacing(14)
        return page, form

    def _path_row(self, key: str, directory: bool = False, file_filter: str = "All files (*)"):
        from PySide6.QtWidgets import QWidget

        wrap = QWidget()
        row = QHBoxLayout(wrap)
        row.setContentsMargins(0, 0, 0, 0)
        edit = LineEdit()
        edit.setText(str(self.cfg.get(key, "")))
        button = PushButton(FluentIcon.FOLDER, tr("choose"))

        def browse():
            current = edit.text() or str(Path.home())
            path = (
                QFileDialog.getExistingDirectory(self, tr("choose_folder"), current)
                if directory
                else QFileDialog.getOpenFileName(self, tr("choose_file"), current, file_filter)[0]
            )
            if path:
                edit.setText(path)

        button.clicked.connect(browse)
        row.addWidget(edit, 1)
        row.addWidget(button)
        edit._container = wrap
        return edit

    def _collect_values(self):
        return {
            "capcut_tts_path": self.capcut_tts_path.text().strip(),
            "device_json_path": self.device_json_path.text().strip(),
            "voice_catalog_url": self.voice_catalog_url.text().strip() or DEFAULT_VOICE_CATALOG_URL,
            "ffprobe_path": self.ffprobe_path.text().strip() or "ffprobe",
            "tts_chunk_size": self.chunk_size.value(),
            "tts_parallel_chunks": self.parallel_chunks.value(),
            "tts_download_workers": self.download_workers.value(),
            "tts_poll_interval_sec": self.poll_interval.value(),
            "max_backups": self.max_backups.value(),
            "language": self.language_combo.currentData() or "vi",
            "theme_mode": self.theme_combo.currentData() or "auto",
        }

    def _save(self):
        self.cfg._data.update(self._collect_values())
        self.cfg._normalize_legacy_keys()
        self.cfg.save()
        self.settings_saved.emit()
        self.accept()

    def _update_voice_catalog(self):
        values = self._collect_values()
        url = values["voice_catalog_url"]
        self.btn_update_voices.setEnabled(False)
        self.lbl_voice_update_status.setText("Đang tải danh sách giọng...")
        QApplication.setOverrideCursor(Qt.WaitCursor)
        try:
            result = update_voice_catalog_from_url(url=url)
        except (VoiceCatalogUpdateError, OSError, ValueError) as exc:
            self.lbl_voice_update_status.setText(f"Tải thất bại: {exc}")
            QMessageBox.warning(self, "Không tải được danh sách giọng", str(exc))
        else:
            self.cfg._data.update(values)
            self.cfg._normalize_legacy_keys()
            self.cfg.save()
            self.settings_saved.emit()
            msg = f"Đã tải {result.voice_count} giọng từ GitHub."
            self.lbl_voice_update_status.setText(msg)
            QMessageBox.information(self, "Đã tải danh sách giọng", msg)
        finally:
            QApplication.restoreOverrideCursor()
            self.btn_update_voices.setEnabled(True)
