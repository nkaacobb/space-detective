"""Step 3 of the detective art pipeline: turn the upscaled crops and masks into clean cutouts.

    python art/finish_parts.py

Keeps only the main paper piece in each mask (dropping labels, arrows and neighbouring parts),
trims to the piece, scales to half of the 4x upscale (2x the study) and writes
assets/detective/<name>.png, plus a review sheet on a navy background.
"""

from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter
from scipy import ndimage

ROOT = Path(__file__).resolve().parent.parent
CUTOUTS = ROOT / "art" / "work" / "cutouts"
OUT = ROOT / "assets" / "detective"
SCALE = 0.5  # of the 4x upscale

# Pieces that must keep more than one blob (e.g. a view with its glass held apart from the body).
KEEP_ALL_LARGE = {"view_hero"}


def clean_alpha(name, rgb, mask):
    a = np.asarray(mask.convert("L"), dtype=np.float32) / 255.0
    solid = a > 0.5
    labels, n = ndimage.label(solid)
    if n == 0:
        return None
    sizes = ndimage.sum(solid, labels, range(1, n + 1))
    largest = sizes.max()
    if name in KEEP_ALL_LARGE:
        keep_ids = [i + 1 for i, s in enumerate(sizes) if s >= largest * 0.04]
    else:
        keep_ids = [int(np.argmax(sizes)) + 1]
    keep = np.isin(labels, keep_ids)
    # Fill pinholes, then grow slightly so the soft mask edge survives the cut.
    keep = ndimage.binary_fill_holes(keep)
    keep = ndimage.binary_dilation(keep, iterations=3)
    alpha = np.where(keep, a, 0.0)
    # Firm up the edge a little: paper cutouts have crisp edges.
    alpha = np.clip((alpha - 0.15) / 0.7, 0.0, 1.0)
    out = rgb.convert("RGBA")
    out.putalpha(Image.fromarray((alpha * 255).astype(np.uint8)))
    return out


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    finished = {}
    for rgb_path in sorted(CUTOUTS.glob("*_rgb.png")):
        name = rgb_path.name[: -len("_rgb.png")]
        rgb = Image.open(rgb_path)
        mask = Image.open(CUTOUTS / f"{name}_mask.png")
        img = clean_alpha(name, rgb, mask)
        if img is None:
            print("empty mask:", name)
            continue
        img = img.crop(img.getbbox())
        img = img.resize((max(1, round(img.width * SCALE)), max(1, round(img.height * SCALE))), Image.LANCZOS)
        img.save(OUT / f"{name}.png", optimize=True)
        finished[name] = img
        print(f"{name}: {img.width}x{img.height}")

    # Review sheet on the app's navy.
    cell = 300
    cols = 6
    rows = (len(finished) + cols - 1) // cols
    sheet = Image.new("RGBA", (cols * cell, rows * (cell + 20)), (24, 36, 64, 255))
    draw = ImageDraw.Draw(sheet)
    for i, (name, img) in enumerate(finished.items()):
        s = min((cell - 12) / img.width, (cell - 12) / img.height, 1.0)
        thumb = img.resize((max(1, int(img.width * s)), max(1, int(img.height * s))), Image.LANCZOS)
        x, y = (i % cols) * cell, (i // cols) * (cell + 20)
        sheet.alpha_composite(thumb, (x + 6, y + 6))
        draw.text((x + 6, y + cell + 2), name, fill=(240, 230, 200, 255))
    sheet.convert("RGB").save(ROOT / "art" / "work" / "parts_sheet.png")


if __name__ == "__main__":
    main()
