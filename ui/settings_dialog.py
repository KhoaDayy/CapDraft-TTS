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


class SettingsDialog(Dialog):
    """Edit settings without exposing the raw JSON file."""

    settings_saved = Signal()

    def __init__(self, parent=None):
        super().__init__(
            "Cài đặt",
            "Tùy chỉnh đường dẫn, danh mục giọng đọc và hiệu năng TTS.",
            parent,
        )
        self.cfg = AppConfig()
        self._build_ui()
        self.yesButton.setText("Lưu cài đặt")
        self.cancelButton.setText("Hủy")
        self.yesButton.clicked.disconnect()
        self.yesButton.clicked.connect(self._save)
        self.setMinimumWidth(680)

    def _build_ui(self):
        tabs = QTabWidget()
        tabs.addTab(self._paths_tab(), "Đường dẫn")
        tabs.addTab(self._voices_tab(), "Giọng đọc")
        tabs.addTab(self._performance_tab(), "Hiệu năng")
        self.textLayout.addWidget(tabs)

    def _paths_tab(self):
        page, form = self._form_page()
        self.capcut_tts_path = self._path_row("capcut_tts_path", directory=True)
        self.device_json_path = self._path_row("device_json_path", file_filter="JSON (*.json)")
        self.ffmpeg_path = LineEdit()
        self.ffmpeg_path.setText(str(self.cfg.get("ffmpeg_path", "ffmpeg")))
        self.ffprobe_path = LineEdit()
        self.ffprobe_path.setText(str(self.cfg.get("ffprobe_path", "ffprobe")))
        form.addRow("CapCut TTS API", self.capcut_tts_path.parentWidget())
        form.addRow("Thiết bị (device.json)", self.device_json_path.parentWidget())
        form.addRow("FFmpeg", self.ffmpeg_path)
        form.addRow("FFprobe", self.ffprobe_path)
        return page

    def _voices_tab(self):
        page, form = self._form_page()
        self.voice_catalog_url = LineEdit()
        self.voice_catalog_url.setText(self.cfg.voice_catalog_url)
        self.voice_catalog_url.setPlaceholderText(DEFAULT_VOICE_CATALOG_URL)
        self.btn_update_voices = PushButton(FluentIcon.SYNC, "Tải lại danh sách")
        self.btn_update_voices.setToolTip("Tải danh sách giọng từ URL (không lưu file local)")
        self.btn_update_voices.clicked.connect(self._update_voice_catalog)
        self.lbl_voice_update_status = QLabel("")
        self.lbl_voice_update_status.setTextInteractionFlags(Qt.TextSelectableByMouse)
        form.addRow("URL danh mục giọng", self.voice_catalog_url)
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
        self.poll_interval.setSuffix(" giây")
        self.poll_interval.setValue(float(self.cfg.get("tts_poll_interval_sec", 1.0)))
        self.max_backups = SpinBox()
        self.max_backups.setRange(1, 100)
        self.max_backups.setValue(int(self.cfg.get("max_backups", 10)))
        form.addRow("Caption mỗi lô", self.chunk_size)
        form.addRow("Số lô song song", self.parallel_chunks)
        form.addRow("Luồng tải audio", self.download_workers)
        form.addRow("Chu kỳ kiểm tra", self.poll_interval)
        form.addRow("Số bản sao lưu", self.max_backups)
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
        button = PushButton(FluentIcon.FOLDER, "Chọn")

        def browse():
            current = edit.text() or str(Path.home())
            path = (
                QFileDialog.getExistingDirectory(self, "Chọn thư mục", current)
                if directory
                else QFileDialog.getOpenFileName(self, "Chọn tệp", current, file_filter)[0]
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
            "ffmpeg_path": self.ffmpeg_path.text().strip() or "ffmpeg",
            "ffprobe_path": self.ffprobe_path.text().strip() or "ffprobe",
            "tts_chunk_size": self.chunk_size.value(),
            "tts_parallel_chunks": self.parallel_chunks.value(),
            "tts_download_workers": self.download_workers.value(),
            "tts_poll_interval_sec": self.poll_interval.value(),
            "max_backups": self.max_backups.value(),
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
