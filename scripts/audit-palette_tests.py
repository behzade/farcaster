import importlib.util
from pathlib import Path
import sys
import unittest


sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("audit_palette", Path(__file__).with_name("audit-palette.py"))
audit = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(audit)


class PaletteAuditTests(unittest.TestCase):
    def test_wcag_reference_values(self):
        self.assertEqual(audit.contrast("#000000", "#ffffff"), 21.0)
        self.assertEqual(audit.contrast("#80786b", "#80786b"), 1.0)
        self.assertAlmostEqual(audit.luminance("#ff0000"), 0.2126)
        self.assertAlmostEqual(audit.luminance("#0a0a0a"), 10 / 255 / 12.92)
        self.assertAlmostEqual(audit.luminance("#0b0b0b"), ((11 / 255 + .055) / 1.055) ** 2.4)

    def test_threshold_uses_unrounded_ratio(self):
        self.assertGreater(audit.contrast("#767676", "#ffffff"), 4.5)
        self.assertLess(audit.contrast("#777777", "#ffffff"), 4.5)

    def test_missing_duplicate_and_translucent_tokens_fail(self):
        css = audit.DEFAULT.read_text()
        for invalid in (css.replace("--text:", "--missing-text:"),
                        css.replace("--muted:", "--text:"),
                        css.replace("--text: #d6caad", "--text: #d6caad80")):
            with self.subTest(css=invalid[:40]), self.assertRaises(ValueError):
                audit.read_colors(invalid)

    def test_real_palette_meets_opaque_token_targets(self):
        colors = audit.read_colors(audit.DEFAULT.read_text())
        failures = [row for row in audit.checks(colors) if not row["meets_target"]]
        self.assertEqual(failures, [])
        counts = audit.summary(audit.checks(colors))["groups"]
        self.assertEqual(counts["text"], len(audit.TEXT) * len(audit.SURFACES))
        self.assertGreater(counts["syntax"], 200)
        self.assertGreater(counts["icon"], 150)

    def test_terminal_green_has_a_legacy_fallback_and_its_own_contrast_check(self):
        css = audit.DEFAULT.read_text()
        colors = audit.read_colors(css)
        self.assertNotEqual(colors["terminal_green"], colors["success"])
        old = audit.read_colors(css.replace(f"  --terminal_green: {colors['terminal_green']};\n", ""))
        self.assertEqual(old["terminal_green"], old["success"])
        colors["terminal_green"] = colors["canvas"]
        self.assertTrue(any(row["group"] == "terminal" and not row["meets_target"]
                            for row in audit.checks(colors)))


if __name__ == "__main__":
    unittest.main()
