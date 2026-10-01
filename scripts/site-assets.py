"""Turns the off-screen renders into the website's images.

    cargo test shots -- --ignored
    python scripts/site-assets.py

Reads target/shots/ (transparent, 2x): site/clock-*.png for the skin
picker, the board, strip, timer and stopwatch scenes, and the sixty ring
frames. Writes site/img/*.webp (lossless, so the pixels stay exact) and
site/og.png, the board on a dark gradient for link previews. Prints each
file's size and the width/height the HTML uses (half the pixels), and
exits 1 if a source is missing or an image is over its budget.
Needs Pillow.
"""

import sys
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
SHOTS = ROOT / "target" / "shots"
SITE = ROOT / "site"
IMG = SITE / "img"

FACES = ["sans", "digital", "matrix"]
PALETTES = ["default", "warm", "cool", "amber", "green", "red", "yellow", "studio"]
MODES = {"board": "mode-board", "board-strip": "mode-strip", "timer": "mode-timer", "stopwatch": "mode-stopwatch"}

TOP, BOTTOM = (22, 24, 28), (8, 9, 11)
OG_SIZE, OG_FIT = (1200, 630), (1040, 510)

KB = 1024
BUDGET = 100 * KB
BUDGETS = {"hero-ring-anim.webp": 600 * KB, "og.png": 200 * KB}
SITE_BUDGET = 2.5 * KB * KB
WEBP = {"lossless": True, "quality": 100, "method": 6}


# Same as gradient() in compose-shots.py, whose name cannot be imported.
def gradient(size):
    w, h = size
    img = Image.new("RGB", size)
    draw = ImageDraw.Draw(img)
    for y in range(h):
        t = y / max(h - 1, 1)
        draw.line([(0, y), (w, y)], fill=tuple(round(a + (b - a) * t) for a, b in zip(TOP, BOTTOM)))
    return img


def load(rel):
    path = SHOTS / rel
    if not path.is_file():
        sys.exit(f"missing {path.relative_to(ROOT).as_posix()}: run `cargo test shots -- --ignored` first")
    return Image.open(path).convert("RGBA")


def og(board):
    canvas = gradient(OG_SIZE).convert("RGBA")
    fit = min(OG_FIT[0] / board.width, OG_FIT[1] / board.height)
    board = board.resize((round(board.width * fit), round(board.height * fit)), Image.LANCZOS)
    canvas.alpha_composite(board, ((OG_SIZE[0] - board.width) // 2, (OG_SIZE[1] - board.height) // 2))
    return canvas.convert("RGB")


def main():
    # Load everything first, so a missing render stops before anything is written.
    skins = {(f, p): load(f"site/clock-{f}-{p}.png") for f in FACES for p in PALETTES}
    modes = {name: load(f"{name}.png") for name in MODES}
    frames = [load(f"ring-{s:02}.png") for s in [*range(37, 60), *range(0, 37)]]
    if len({frame.size for frame in frames}) != 1:
        sys.exit("the ring frames differ in size")

    IMG.mkdir(parents=True, exist_ok=True)
    written = []

    def save(img, path, **extra):
        img.save(path, **extra)
        written.append((path, img.size))

    for (face, palette), img in skins.items():
        save(img, IMG / f"skin-{face}-{palette}.webp", format="WEBP", **WEBP)
    for name, img in modes.items():
        save(img, IMG / f"{MODES[name]}.webp", format="WEBP", **WEBP)
    # The static hero is the animation's first frame, so swapping one for the other does not jump.
    save(frames[0], IMG / "hero-ring.webp", format="WEBP", **WEBP)
    frames[0].save(
        IMG / "hero-ring-anim.webp", format="WEBP", save_all=True, append_images=frames[1:],
        duration=1000, loop=0, **WEBP,
    )
    written.append((IMG / "hero-ring-anim.webp", frames[0].size))
    save(og(modes["board"]), SITE / "og.png", format="PNG", optimize=True)

    over = []
    print(f"{'file':<32} {'px':>10} {'bytes':>9}  html w x h")
    for path, (w, h) in written:
        size = path.stat().st_size
        rel = path.relative_to(SITE).as_posix()
        html = "-" if path.name == "og.png" else f"{w // 2} x {h // 2}"
        print(f"{rel:<32} {f'{w}x{h}':>10} {size:>9}  {html}")
        if size > BUDGETS.get(path.name, BUDGET):
            over.append(rel)
    total = sum(p.stat().st_size for p in SITE.rglob("*") if p.is_file())
    print(f"site/ total: {total} bytes")
    if total > SITE_BUDGET:
        over.append("site/ as a whole")
    if over:
        sys.exit("over budget: " + ", ".join(over))


if __name__ == "__main__":
    main()
