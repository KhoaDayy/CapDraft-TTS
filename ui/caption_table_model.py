"""Virtualized caption table model for large CapCut projects (thousands of rows)."""

from __future__ import annotations

from dataclasses import dataclass, field

from PySide6.QtCore import (
    QAbstractTableModel,
    QModelIndex,
    QSortFilterProxyModel,
    Qt,
)

from core.capcut_project.models import CaptionRow
from core.i18n import tr


def _fmt_us(us: int) -> str:
    total_ms = max(0, int(round(us / 1000.0)))
    hours, rem = divmod(total_ms, 3_600_000)
    minutes, rem = divmod(rem, 60_000)
    secs, ms = divmod(rem, 1000)
    if hours:
        return f"{hours:02d}:{minutes:02d}:{secs:02d}.{ms:03d}"
    return f"{minutes:02d}:{secs:02d}.{ms:03d}"


@dataclass
class CaptionRowState:
    caption: CaptionRow
    checked: bool = True
    status: str = ""
    duration: str = ""
    error: str = ""
    # Cached display strings (avoid reformat on every paint)
    start_txt: str = ""
    end_txt: str = ""
    text_txt: str = ""
    has_tts_txt: str = ""
    text_lower: str = field(default="", repr=False)

    def __post_init__(self):
        cap = self.caption
        self.start_txt = _fmt_us(cap.start_us)
        self.end_txt = _fmt_us(cap.end_us)
        self.text_txt = (cap.text or "").replace("\n", " ")
        self.text_lower = self.text_txt.lower()
        if not self.status:
            if cap.is_empty:
                self.status = "empty"
                self.checked = False
            elif cap.has_existing_tts:
                self.status = "has_tts"
            else:
                self.status = "ready"


class CaptionTableModel(QAbstractTableModel):
    COL_CHECK = 0
    COL_INDEX = 1
    COL_START = 2
    COL_END = 3
    COL_TEXT = 4
    COL_HAS_TTS = 5
    COL_STATUS = 6
    COL_DURATION = 7
    COL_ERROR = 8
    COLUMN_COUNT = 9

    def __init__(self, parent=None):
        super().__init__(parent)
        self._rows: list[CaptionRowState] = []
        self._index_to_row: dict[int, int] = {}
        self._headers = ["", "#", "", "", "", "", "", "", ""]
        self._checks_enabled = True
        self.refresh_headers()

    def refresh_headers(self) -> None:
        self._headers = [
            "",
            "#",
            tr("table_start"),
            tr("table_end"),
            tr("table_content"),
            tr("table_has_tts"),
            tr("table_status"),
            tr("table_duration"),
            tr("table_error"),
        ]
        self.headerDataChanged.emit(Qt.Horizontal, 0, self.COLUMN_COUNT - 1)

    def set_captions(self, captions: list[CaptionRow]) -> None:
        self.beginResetModel()
        yes_txt, no_txt = tr("yes"), tr("no")
        rows: list[CaptionRowState] = []
        index_map: dict[int, int] = {}
        for i, cap in enumerate(captions):
            state = CaptionRowState(caption=cap)
            state.has_tts_txt = yes_txt if cap.has_existing_tts else no_txt
            rows.append(state)
            index_map[cap.index] = i
        self._rows = rows
        self._index_to_row = index_map
        self.endResetModel()

    def clear(self) -> None:
        self.beginResetModel()
        self._rows = []
        self._index_to_row = {}
        self.endResetModel()

    def rowCount(self, parent=QModelIndex()) -> int:  # noqa: N802
        if parent.isValid():
            return 0
        return len(self._rows)

    def columnCount(self, parent=QModelIndex()) -> int:  # noqa: N802
        if parent.isValid():
            return 0
        return self.COLUMN_COUNT

    def headerData(self, section, orientation, role=Qt.DisplayRole):  # noqa: N802
        if orientation == Qt.Horizontal and role == Qt.DisplayRole:
            if 0 <= section < len(self._headers):
                return self._headers[section]
        return None

    def flags(self, index):  # noqa: N802
        if not index.isValid():
            return Qt.NoItemFlags
        flags = Qt.ItemIsEnabled | Qt.ItemIsSelectable
        if index.column() == self.COL_CHECK and self._checks_enabled:
            flags |= Qt.ItemIsUserCheckable
        return flags

    def set_checks_enabled(self, enabled: bool) -> None:
        if self._checks_enabled == enabled:
            return
        self._checks_enabled = enabled
        if self._rows:
            top = self.index(0, self.COL_CHECK)
            bottom = self.index(len(self._rows) - 1, self.COL_CHECK)
            self.dataChanged.emit(top, bottom, [Qt.ItemDataRole.CheckStateRole])

    def data(self, index, role=Qt.DisplayRole):  # noqa: N802
        if not index.isValid():
            return None
        row = index.row()
        col = index.column()
        if row < 0 or row >= len(self._rows):
            return None
        state = self._rows[row]
        cap = state.caption

        if role == Qt.CheckStateRole and col == self.COL_CHECK:
            return Qt.Checked if state.checked else Qt.Unchecked

        if role == Qt.UserRole and col == self.COL_INDEX:
            return cap.text_segment_id

        if role == Qt.UserRole + 1:
            return state

        if role == Qt.DisplayRole:
            if col == self.COL_CHECK:
                return ""
            if col == self.COL_INDEX:
                return str(cap.index)
            if col == self.COL_START:
                return state.start_txt
            if col == self.COL_END:
                return state.end_txt
            if col == self.COL_TEXT:
                return state.text_txt
            if col == self.COL_HAS_TTS:
                return state.has_tts_txt
            if col == self.COL_STATUS:
                return self._status_label(state.status)
            if col == self.COL_DURATION:
                return state.duration
            if col == self.COL_ERROR:
                return state.error
        return None

    def setData(self, index, value, role=Qt.EditRole):  # noqa: N802
        if not index.isValid():
            return False
        row = index.row()
        if row < 0 or row >= len(self._rows):
            return False
        state = self._rows[row]
        if role == Qt.CheckStateRole and index.column() == self.COL_CHECK:
            if not self._checks_enabled:
                return False
            state.checked = value == Qt.Checked
            self.dataChanged.emit(index, index, [Qt.ItemDataRole.CheckStateRole])
            return True
        return False

    @staticmethod
    def _status_label(status: str) -> str:
        # i18n keys where they exist; short runtime statuses stay language-neutral.
        if status == "empty":
            return tr("empty")
        if status == "has_tts":
            return tr("has_tts")
        if status == "ready":
            return tr("ready")
        if status == "failed":
            return tr("error")
        if status == "cached":
            return "Cache"
        if status == "generated":
            return "OK"
        return status

    def row_state(self, row: int) -> CaptionRowState | None:
        if 0 <= row < len(self._rows):
            return self._rows[row]
        return None

    def source_row_for_caption_index(self, caption_index: int) -> int | None:
        return self._index_to_row.get(int(caption_index))

    def apply_item_results(self, updates: dict[int, dict]) -> None:
        """Batch-apply TTS item results keyed by caption index."""
        if not updates:
            return
        changed_rows: list[int] = []
        for idx, res in updates.items():
            row = self._index_to_row.get(int(idx))
            if row is None:
                continue
            state = self._rows[row]
            status = res.get("status", "")
            if status == "Cached":
                state.status = "cached"
            elif status == "Failed":
                state.status = "failed"
                state.error = str(res.get("error") or "")
            else:
                state.status = "generated"
            dur = float(res.get("duration") or 0.0)
            if dur > 0:
                state.duration = f"{dur:.2f}s"
            changed_rows.append(row)
        if not changed_rows:
            return
        lo, hi = min(changed_rows), max(changed_rows)
        top = self.index(lo, self.COL_STATUS)
        bottom = self.index(hi, self.COL_ERROR)
        self.dataChanged.emit(top, bottom, [Qt.ItemDataRole.DisplayRole])

    def set_all_checked(self, on: bool, *, only_rows: list[int] | None = None) -> None:
        targets = only_rows if only_rows is not None else list(range(len(self._rows)))
        if not targets:
            return
        for row in targets:
            state = self._rows[row]
            if on and state.caption.is_empty:
                continue
            state.checked = on
        lo, hi = min(targets), max(targets)
        top = self.index(lo, self.COL_CHECK)
        bottom = self.index(hi, self.COL_CHECK)
        self.dataChanged.emit(top, bottom, [Qt.ItemDataRole.CheckStateRole])

    def selected_segment_ids(self) -> list[str]:
        out: list[str] = []
        for state in self._rows:
            if state.checked and state.caption.text_segment_id:
                out.append(state.caption.text_segment_id)
        return out

    def selection_counts(self, visible_source_rows: set[int] | None = None) -> tuple[int, int, int]:
        """Return (selected, total, visible)."""
        total = len(self._rows)
        selected = sum(1 for s in self._rows if s.checked)
        if visible_source_rows is None:
            visible = total
        else:
            visible = len(visible_source_rows)
        return selected, total, visible


class CaptionFilterProxy(QSortFilterProxyModel):
    def __init__(self, parent=None):
        super().__init__(parent)
        self._query = ""
        self._hide_empty = True
        self._only_no_tts = False
        self._only_errors = False
        self.setDynamicSortFilter(True)

    def set_filters(
        self,
        *,
        query: str = "",
        hide_empty: bool = True,
        only_no_tts: bool = False,
        only_errors: bool = False,
    ) -> None:
        self._query = (query or "").strip().lower()
        self._hide_empty = hide_empty
        self._only_no_tts = only_no_tts
        self._only_errors = only_errors
        # Qt6.10+: invalidateRowsFilter; fall back for older bindings.
        if hasattr(self, "invalidateRowsFilter"):
            self.invalidateRowsFilter()
        else:
            self.invalidateFilter()

    def filterAcceptsRow(self, source_row: int, source_parent: QModelIndex) -> bool:  # noqa: N802
        model = self.sourceModel()
        if not isinstance(model, CaptionTableModel):
            return True
        state = model.row_state(source_row)
        if state is None:
            return False
        cap = state.caption
        if self._query and self._query not in state.text_lower:
            return False
        if self._hide_empty and (cap.is_empty or state.status == "empty"):
            return False
        if self._only_no_tts and cap.has_existing_tts:
            return False
        if self._only_errors:
            is_err = state.status == "failed" or bool(state.error.strip())
            if not is_err:
                return False
        return True

    def visible_source_rows(self) -> set[int]:
        rows: set[int] = set()
        for proxy_row in range(self.rowCount()):
            src = self.mapToSource(self.index(proxy_row, 0))
            if src.isValid():
                rows.add(src.row())
        return rows
