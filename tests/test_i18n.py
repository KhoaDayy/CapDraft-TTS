import unittest

from core.i18n import SUPPORTED_LANGUAGES, normalize_language, translate
from core.preferences import normalize_theme_mode


class TestI18n(unittest.TestCase):
    def test_four_languages_are_supported(self):
        self.assertEqual(tuple(SUPPORTED_LANGUAGES), ("vi", "en", "zh", "ja"))

    def test_core_ui_text_is_translated_in_all_languages(self):
        expected = {
            "vi": "Chọn project",
            "en": "Choose project",
            "zh": "选择项目",
            "ja": "プロジェクトを選択",
        }
        for language, text in expected.items():
            self.assertEqual(translate("choose_project", language=language), text)

    def test_unknown_language_falls_back_to_vietnamese(self):
        self.assertEqual(normalize_language("de"), "vi")
        self.assertEqual(translate("settings", language="de"), "Cài đặt")


class TestAppearancePreferences(unittest.TestCase):
    def test_theme_accepts_auto_light_and_dark(self):
        for mode in ("auto", "light", "dark"):
            self.assertEqual(normalize_theme_mode(mode), mode)

    def test_unknown_theme_falls_back_to_auto(self):
        self.assertEqual(normalize_theme_mode("neon"), "auto")


if __name__ == "__main__":
    unittest.main()
