"""Puts the off-screen renders from `cargo test shots -- --ignored` on a backdrop.

    python scripts/compose-shots.py [--wallpaper path/to/image.png]

Reads target/shots/ (transparent, 2x) and writes docs/screenshots/ for the
README: one PNG per scene on a dark gradient, faces.png with every face in a
spread of colours, ring.gif from the sixty ring frames, and hero.png: the
board over a wallpaper, if one is given, as it sits on a desktop.
Needs Pillow.
"""

import argparse
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
SHOTS = ROOT / "target" / "shots"
OUT = ROOT / "docs" / "screenshots"
# Output name -> render, chosen so the README shows every face and a spread
# of colours rather than one look over and over.
SCENES = {
    "board": "site/mode-board",
    "strip": "site/mode-strip",
    "clock": "site/mode-clock",
    "timer": "site/mode-timer",
    "stopwatch": "site/mode-stopwatch",
    "look-12h": "site/look-12h",
    "look-night": "site/look-night",
    "look-chroma": "site/look-chroma",
    "look-small": "site/look-small",
}
# Rows of faces.png: each face in four colours, all eight palettes between them.
FACES = [
    ("sans", ["default", "warm", "cool", "yellow"]),
    ("digital", ["red", "amber", "green", "studio"]),
    ("matrix", ["green", "red", "amber", "cool"]),
]
TOP, BOTTOM = (22, 24, 28), (8, 9, 11)
CHROMA = (0, 255, 0)


def gradient(size):
    w, h = size
    img = Image.new("RGB", size)
    draw = ImageDraw.Draw(img)
    for y in range(h):
        t = y / max(h - 1, 1)
        draw.line([(0, y), (w, y)], fill=tuple(round(a + (b - a) * t) for a, b in zip(TOP, BOTTOM)))
    return img


def on_gradient(shot, margin, background=None):
    size = (shot.width + 2 * margin, shot.height + 2 * margin)
    canvas = (Image.new("RGB", size, background) if background else gradient(size)).convert("RGBA")
    canvas.alpha_composite(shot, (margin, margin))
    return canvas.convert("RGB")


def faces_grid(margin=48, gap=24):
    """Every face in four colours each, centred in equal cells. Studio gets
    its seconds ring, without which it looks just like Green."""
    rows = []
    for face, palettes in FACES:
        row = []
        for palette in palettes:
            ring = 1 if palette == "studio" else 0
            row.append(Image.open(SHOTS / "site" / f"skin-{face}-{palette}-r{ring}-b0.png").convert("RGBA"))
        rows.append(row)
    cell_w = max(img.width for row in rows for img in row)
    cell_h = max(img.height for row in rows for img in row)
    cols = max(len(row) for row in rows)
    width = 2 * margin + cols * cell_w + (cols - 1) * gap
    height = 2 * margin + len(rows) * cell_h + (len(rows) - 1) * gap
    canvas = gradient((width, height)).convert("RGBA")
    for r, row in enumerate(rows):
        for c, img in enumerate(row):
            x = margin + c * (cell_w + gap) + (cell_w - img.width) // 2
            y = margin + r * (cell_h + gap) + (cell_h - img.height) // 2
            canvas.alpha_composite(img, (x, y))
    # Half size: the README shows it about 860 px wide, so this stays sharp
    # on a high-density screen without a 3000 px file.
    canvas = canvas.resize((width // 2, height // 2), Image.LANCZOS)
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

    for name, source in SCENES.items():
        shot = Image.open(SHOTS / f"{source}.png").convert("RGBA")
        # The renderer leaves the chroma key's green to the window's clear
        # colour, which an off-screen render does not have.
        background = CHROMA if name == "look-chroma" else None
        on_gradient(shot, 48, background).save(OUT / f"{name}.png", optimize=True)
        print(OUT / f"{name}.png")

    faces_grid().save(OUT / "faces.png", optimize=True)
    print(OUT / "faces.png")

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
