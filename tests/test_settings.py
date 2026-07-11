"""Settings dialog smoke tests."""

import os
import unittest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PySide6.QtWidgets import QApplication, QTabWidget

from ui.settings_dialog import SettingsDialog


class TestSettingsDialog(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def test_dialog_builds_all_sections(self):
        dialog = SettingsDialog()
        self.assertTrue(dialog.capcut_tts_path.text())
        self.assertEqual(dialog.findChild(QTabWidget).count(), 3)
        self.assertIn("githubusercontent.com", dialog.voice_catalog_update_url.text())
        self.assertGreaterEqual(dialog.chunk_size.value(), 1)
        dialog.close()


if __name__ == "__main__":
    unittest.main()
