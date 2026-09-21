"""Puts the off-screen renders from `cargo test shots -- --ignored` on a backdrop.

    python scripts/compose-shots.py [--wallpaper path/to/image.png]

Reads target/shots/*.png (transparent, 2x) and writes docs/screenshots/:
one PNG per scene on a dark gradient, ring.gif from the sixty ring frames,
and hero.png: the board over a wallpaper, if one is given, as it sits on a desktop.
Needs Pillow.
"""

import argparse
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parent.parent
SHOTS = ROOT / "target" / "shots"
OUT = ROOT / "docs" / "screenshots"
SCENES = ["board", "board-strip", "clock-ring", "clock-segment", "clock-sans", "timer", "stopwatch"]
TOP, BOTTOM = (22, 24, 28), (8, 9, 11)


def gradient(size):
    w, h = size
    img = Image.new("RGB", size)
    draw = ImageDraw.Draw(img)
    for y in range(h):
        t = y / max(h - 1, 1)
        draw.line([(0, y), (w, y)], fill=tuple(round(a + (b - a) * t) for a, b in zip(TOP, BOTTOM)))
    return img


def on_gradient(shot, margin):
    canvas = gradient((shot.width + 2 * margin, shot.height + 2 * margin)).convert("RGBA")
    canvas.alpha_composite(shot, (margin, margin))
    return canvas.convert("RGB")


def hero(board, wallpaper_path, width=1600, height=900):
    wall = Image.open(wallpaper_path).convert("RGB")
    scale = max(width / wall.width, height / wall.height)
    wall = wall.resize((round(wall.width * scale), round(wall.height * scale)), Image.LANCZOS)
    left, top = (wall.width - width) // 2, (wall.height - height) // 2
    canvas = wall.crop((left, top, left + width, top + height)).convert("RGBA")
    # Where an overlay usually lives: the top-right corner, clear of the edges.
    fit = min(1.0, (width * 0.42) / board.width)
    board = board.resize((round(board.width * fit), round(board.height * fit)), Image.LANCZOS)
    canvas.alpha_composite(board, (width - board.width - 48, 48))
    return canvas.convert("RGB")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--wallpaper", type=Path)
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)

    for name in SCENES:
        shot = Image.open(SHOTS / f"{name}.png").convert("RGBA")
        on_gradient(shot, 48).save(OUT / f"{name}.png", optimize=True)
        print(OUT / f"{name}.png")

    frames = [Image.open(SHOTS / f"ring-{s:02}.png").convert("RGBA") for s in range(60)]
    if frames:
        gif = [on_gradient(f, 32).resize((f.width // 2 + 32, f.height // 2 + 32), Image.LANCZOS) for f in frames]
        gif = [g.convert("P", palette=Image.ADAPTIVE, colors=64) for g in gif]
        gif[0].save(OUT / "ring.gif", save_all=True, append_images=gif[1:], duration=1000, loop=0, optimize=True)
        print(OUT / "ring.gif")

    if args.wallpaper:
        hero(Image.open(SHOTS / "board.png").convert("RGBA"), args.wallpaper).save(OUT / "hero.png", optimize=True)
        print(OUT / "hero.png")


if __name__ == "__main__":
    main()
