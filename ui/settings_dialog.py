"""Persistent application settings dialog."""

from __future__ import annotations

from pathlib import Path

from PySide6.QtCore import Signal
from PySide6.QtWidgets import QFileDialog, QFormLayout, QHBoxLayout, QLabel, QTabWidget, QVBoxLayout
from qfluentwidgets import (
    ComboBox,
    Dialog,
    DoubleSpinBox,
    FluentIcon,
    LineEdit,
    PushButton,
    SpinBox,
)

from core.config import AppConfig


class SettingsDialog(Dialog):
    """Edit settings without exposing the raw JSON file."""

    settings_saved = Signal()

    def __init__(self, parent=None):
        super().__init__("Cài đặt", "Tùy chỉnh giao diện, đường dẫn và hiệu năng TTS.", parent)
        self.cfg = AppConfig()
        self._build_ui()
        self.yesButton.setText("Lưu cài đặt")
        self.cancelButton.setText("Hủy")
        self.yesButton.clicked.disconnect()
        self.yesButton.clicked.connect(self._save)
        self.setMinimumWidth(620)

    def _build_ui(self):
        tabs = QTabWidget()
        tabs.addTab(self._appearance_tab(), "Giao diện")
        tabs.addTab(self._paths_tab(), "Đường dẫn")
        tabs.addTab(self._performance_tab(), "Hiệu năng")
        self.textLayout.addWidget(tabs)

    def _appearance_tab(self):
        page, form = self._form_page()
        self.theme = ComboBox()
        for label, value in (("Theo hệ thống", "auto"), ("Sáng", "light"), ("Tối", "dark")):
            self.theme.addItem(label, userData=value)
        index = max(0, self.theme.findData(self.cfg.get("theme", "auto")))
        self.theme.setCurrentIndex(index)
        self.accent = LineEdit()
        self.accent.setText(str(self.cfg.get("accent_color", "#0EA5A4")))
        self.accent.setPlaceholderText("#0EA5A4")
        form.addRow("Chủ đề", self.theme)
        form.addRow("Màu nhấn", self.accent)
        form.addRow("", QLabel("Thay đổi giao diện được áp dụng ngay sau khi lưu."))
        return page

    def _paths_tab(self):
        page, form = self._form_page()
        self.capcut_tts_path = self._path_row("capcut_tts_path", directory=True)
        self.device_json_path = self._path_row("device_json_path", file_filter="JSON (*.json)")
        self.voice_catalog_path = self._path_row("voice_catalog_path", file_filter="JSON (*.json)")
        self.ffmpeg_path = LineEdit()
        self.ffmpeg_path.setText(str(self.cfg.get("ffmpeg_path", "ffmpeg")))
        self.ffprobe_path = LineEdit()
        self.ffprobe_path.setText(str(self.cfg.get("ffprobe_path", "ffprobe")))
        form.addRow("CapCut TTS API", self.capcut_tts_path.parentWidget())
        form.addRow("Thiết bị (device.json)", self.device_json_path.parentWidget())
        form.addRow("Danh mục giọng", self.voice_catalog_path.parentWidget())
        form.addRow("FFmpeg", self.ffmpeg_path)
        form.addRow("FFprobe", self.ffprobe_path)
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
            path = (QFileDialog.getExistingDirectory(self, "Chọn thư mục", current) if directory
                    else QFileDialog.getOpenFileName(self, "Chọn tệp", current, file_filter)[0])
            if path:
                edit.setText(path)
        button.clicked.connect(browse)
        row.addWidget(edit, 1)
        row.addWidget(button)
        edit._container = wrap
        return edit

    def _save(self):
        values = {
            "theme": self.theme.currentData(),
            "accent_color": self.accent.text().strip() or "#0EA5A4",
            "capcut_tts_path": self.capcut_tts_path.text().strip(),
            "device_json_path": self.device_json_path.text().strip(),
            "voice_catalog_path": self.voice_catalog_path.text().strip(),
            "ffmpeg_path": self.ffmpeg_path.text().strip() or "ffmpeg",
            "ffprobe_path": self.ffprobe_path.text().strip() or "ffprobe",
            "tts_chunk_size": self.chunk_size.value(),
            "tts_parallel_chunks": self.parallel_chunks.value(),
            "tts_download_workers": self.download_workers.value(),
            "tts_poll_interval_sec": self.poll_interval.value(),
            "max_backups": self.max_backups.value(),
        }
        self.cfg._data.update(values)
        self.cfg.save()
        self.settings_saved.emit()
        self.accept()
