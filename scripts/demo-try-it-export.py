#!/usr/bin/env python3
"""Export real camera terminal cells, cropping tab chrome and redacting host paths.

Requires pyte 0.8.2. Raw camera output remains private. At 12 fps, replay actual
client output and emit only changed rows, retaining cell colors and cursor state.
No discussion text is supplied by this exporter. Home-prefix substitutions keep
exact cell width, including prefixes wrapped across rows inside a pane.
"""
import argparse
import json
from pathlib import Path
import re

import pyte


class Screen(pyte.Screen):
    # Device queries do not affect visible cells; private DSR is unsupported by
    # pyte's stock handler. Never feed terminal replies into the recorded client.
    def report_device_status(self, *args, **kwargs):
        pass

    def report_device_attributes(self, *args, **kwargs):
        pass


COLORS = dict(zip(["black", "red", "green", "brown", "blue", "magenta", "cyan", "white"], range(8)))


def style(char):
    values = ["0"]
    for name, code in [("bold", "1"), ("italics", "3"), ("underscore", "4"), ("reverse", "7"), ("strikethrough", "9")]:
        if getattr(char, name, False):
            values.append(code)
    for color, offset in [(char.fg, 30), (char.bg, 40)]:
        if color in COLORS:
            values.append(str(offset + COLORS[color]))
        elif re.fullmatch(r"[0-9a-fA-F]{6}", color):
            values.extend([str(offset + 8), "2", *[str(int(color[n:n+2], 16)) for n in (0, 2, 4)]])
        elif color.startswith("bright") and color[6:] in COLORS:
            values.append(str(offset + 60 + COLORS[color[6:]]))
        elif color != "default":
            raise ValueError(f"unsupported terminal color {color!r}")
    return "\x1b[" + ";".join(values) + "m"


def rows(screen, home):
    grid = [[screen.buffer[y][x] for x in range(screen.columns)] for y in range(screen.lines)]
    labels = screen.display
    bottom = next((y for y, line in enumerate(labels) if line.startswith("┌ you ")), None)
    split = screen.columns // 2
    regions = [(1, screen.lines, 0, screen.columns)] if bottom is None else [
        (2, bottom - 1, 1, split - 1), (2, bottom - 1, split + 1, screen.columns - 1),
        (bottom + 1, screen.lines - 1, 1, screen.columns - 1)]
    replacement = "$HOST_HOME".ljust(len(home), "_")
    if len(replacement) != len(home):
        raise ValueError("home prefix must have at least ten characters")
    for top, end, left, right in regions:
        pairs = [(char, (y, x)) for y in range(top, end) for x in range(left, right)
                 for char in grid[y][x].data if not char.isspace()]
        positions = [position for _, position in pairs]
        text = "".join(char for char, _ in pairs)
        for match in re.finditer(re.escape(home), text):
            for (y, x), char in zip(positions[match.start():match.end()], replacement):
                grid[y][x] = grid[y][x]._replace(data=char)
        # A wrapped prefix can scroll above the viewport, leaving only the
        # personal directory name. Mask that orphan without altering cell width.
        name = Path(home).name
        for match in re.finditer(re.escape(name), text):
            selected = positions[match.start():match.end()]
            if "".join(grid[y][x].data for y, x in selected) == name:
                for (y, x), char in zip(selected, "$USER".ljust(len(name), "_")):
                    grid[y][x] = grid[y][x]._replace(data=char)
    # The actual client tab bar is UI chrome; omit it, preserving every pane.
    return grid[1:]


def export(source, target, home):
    events = [json.loads(line) for line in source.read_text().splitlines()]
    header = dict(events[0]); header["height"] -= 1
    screen = Screen(events[0]["width"], events[0]["height"])
    stream = pyte.Stream(screen)
    index = 1
    previous = None
    previous_cursor = None
    with target.open("x") as output:
        output.write(json.dumps(header) + "\n")
        duration = events[-1][0]
        tick = 0
        while tick / 12 <= duration + 1 / 12:
            timestamp = min(tick / 12, duration)
            while index < len(events) and events[index][0] <= timestamp:
                if events[index][1] == "o":
                    stream.feed(events[index][2])
                index += 1
            cells = rows(screen, home)
            chunk = "\x1b[?25l" + ("\x1b[2J" if previous is None else "")
            changed = False
            for y, line in enumerate(cells):
                if previous is not None and previous[y] == line:
                    continue
                changed = True
                chunk += f"\x1b[{y+1};1H"
                current_style = None
                for char in line:
                    attributes = style(char)
                    if attributes != current_style:
                        chunk += attributes
                        current_style = attributes
                    chunk += char.data
            chunk += "\x1b[0m"
            if not screen.cursor.hidden and screen.cursor.y > 0:
                chunk += f"\x1b[{screen.cursor.y};{screen.cursor.x+1}H\x1b[?25h"
            cursor = (screen.cursor.x, screen.cursor.y, screen.cursor.hidden)
            if changed or cursor != previous_cursor or timestamp == duration:
                output.write(json.dumps([round(timestamp, 6), "o", chunk]) + "\n")
            previous = cells
            previous_cursor = cursor
            if timestamp == duration:
                break
            tick += 1


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("target", type=Path)
    parser.add_argument("--home-prefix", required=True)
    args = parser.parse_args()
    export(args.source, args.target, args.home_prefix)
