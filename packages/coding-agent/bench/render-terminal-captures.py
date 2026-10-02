"""Rasterize the real VT cell captures emitted by session-terminal-navigation.test.ts.

Usage: python bench/render-terminal-captures.py CAPTURE_DIRECTORY
Requires Pillow. These are virtual-terminal screenshots, not desktop captures.
"""

import json
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


def color(word: int, default: tuple[int, int, int]) -> tuple[int, int, int]:
    kind, value = word >> 24, word & 0xFFFFFF
    if kind == 2:
        return ((value >> 16) & 255, (value >> 8) & 255, value & 255)
    if kind == 1:
        palette = [
            (0, 0, 0), (205, 49, 49), (13, 188, 121), (229, 229, 16),
            (36, 114, 200), (188, 63, 188), (17, 168, 205), (229, 229, 229),
            (102, 102, 102), (241, 76, 76), (35, 209, 139), (245, 245, 67),
            (59, 142, 234), (214, 112, 214), (41, 184, 219), (255, 255, 255),
        ]
        if value < 16:
            return palette[value]
        if value < 232:
            index = value - 16
            cube = [0, 95, 135, 175, 215, 255]
            return (cube[index // 36], cube[(index // 6) % 6], cube[index % 6])
        gray = min(255, 8 + 10 * (value - 232))
        return (gray, gray, gray)
    return default


def render(source: Path) -> Path:
    capture = json.loads(source.read_text(encoding="utf-8"))
    font = ImageFont.truetype("C:/Windows/Fonts/consola.ttf", 18)
    bold = ImageFont.truetype("C:/Windows/Fonts/consolab.ttf", 18)
    cell_width, cell_height, padding, header = 11, 24, 18, 36
    image = Image.new("RGB", (capture["columns"] * cell_width + 2 * padding,
                              capture["rows"] * cell_height + 2 * padding + header), (18, 20, 25))
    draw = ImageDraw.Draw(image)
    draw.text((padding, 8), "VT screenshot: " + capture["label"], font=font, fill=(170, 184, 202))
    for row, words in enumerate(capture["cells"]):
        for column in range(len(words) // 8):
            char, combining1, combining2, foreground, background, _, flags, _ = words[column * 8:column * 8 + 8]
            fg, bg = color(foreground, (230, 230, 230)), color(background, (18, 20, 25))
            if flags & 8:
                fg, bg = bg, fg
            if flags & 2:
                fg = tuple((component + bg[index]) // 2 for index, component in enumerate(fg))
            x, y = padding + column * cell_width, padding + header + row * cell_height
            draw.rectangle((x, y, x + cell_width - 1, y + cell_height - 1), fill=bg)
            if flags & 128:
                continue
            text = "".join(chr(point) for point in (char, combining1, combining2) if 0 < point <= 0x10FFFF)
            draw.text((x, y), text, font=bold if flags & 1 else font, fill=fg)
            if flags & 0x700:
                draw.line((x, y + cell_height - 3, x + cell_width, y + cell_height - 3), fill=fg)
    target = source.with_suffix(".png")
    image.save(target)
    return target


if __name__ == "__main__":
    for capture_path in sorted(Path(sys.argv[1]).glob("*.json")):
        if "cells" in json.loads(capture_path.read_text(encoding="utf-8")):
            print(render(capture_path).resolve())
