"""Privacy export must preserve all non-redacted terminal cells."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("exporter", Path(__file__).resolve().parents[1] / "demo-try-it-export.py")
exporter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(exporter)


class ExportTests(unittest.TestCase):
    def test_wrapped_path_redaction_keeps_cell_width_colors_and_other_panes(self):
        screen = exporter.Screen(80,12)
        stream = exporter.pyte.Stream(screen)
        stream.feed("\x1b[2;1H┌ alice\x1b[2;41H┌ bob\x1b[9;1H┌ you ")
        stream.feed("\x1b[3;2H\x1b[38;2;12;34;56mTool /Users/\x1b[4;2Halepar/.config/herdr.sock")
        stream.feed("\x1b[3;42HREAL BOB OUTPUT\x1b[10;2Hyou $ follow review")
        before = [[screen.buffer[y][x] for x in range(80)] for y in range(12)]
        result = exporter.rows(screen, "/Users/alepar")
        self.assertEqual(len(result),11)
        changed = [(y+1,x) for y,row in enumerate(result) for x,char in enumerate(row) if char != before[y+1][x]]
        self.assertEqual(len(changed),13)
        for y,x in changed:
            self.assertEqual(result[y-1][x].fg,before[y][x].fg)
            self.assertEqual(result[y-1][x].bg,before[y][x].bg)
        self.assertEqual("".join(c.data for c in result[1][41:56]),"REAL BOB OUTPUT")
        flattened = "".join(c.data for row in result for c in row)
        self.assertNotIn("alepar",flattened)
        self.assertIn("$HOST_H",flattened)

    def test_private_device_queries_do_not_change_visible_cells(self):
        screen = exporter.Screen(80,12)
        stream = exporter.pyte.Stream(screen)
        stream.feed("actual output")
        before = list(screen.display)
        stream.feed("\x1b[?6n\x1b[6n\x1b[c")
        self.assertEqual(screen.display,before)


if __name__ == "__main__":
    unittest.main()
