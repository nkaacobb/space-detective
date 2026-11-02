"""Splits the detective's head into layers so his eyes can follow the mouse and his mouth can move.

    python art/make_eyes.py

Writes assets/detective/head_base.png (pupils painted out with eye white), head_nose.png (just the
nose, drawn over the moving pupils) and head_moustache.png (drawn over the mouth as it opens). The
geometry used here is in rig.json under "eyes", "nose" and "moustache".
"""

import json
from pathlib import Path

import numpy as np
from PIL import Image
from scipy import ndimage

ROOT = Path(__file__).resolve().parent.parent
PARTS = ROOT / "assets" / "detective"


def ellipse_mask(shape, cx, cy, rx, ry, grow=0.0):
    yy, xx = np.mgrid[0:shape[0], 0:shape[1]]
    return ((xx - cx) / (rx + grow)) ** 2 + ((yy - cy) / (ry + grow)) ** 2 <= 1.0


def main():
    rig = json.loads((PARTS / "rig.json").read_text())
    head = np.asarray(Image.open(PARTS / "head.png").convert("RGBA"), dtype=np.float32)
    base = head.copy()
    h, w = head.shape[:2]

    for eye in rig["eyes"]:
        cx, cy = eye["pupil_center"]
        white = ellipse_mask((h, w), *eye["center"], eye["rx"] - 3, eye["ry"] - 3)
        pupil = ellipse_mask((h, w), cx, cy, eye["pupil_rx"], eye["pupil_ry"], grow=4)
        # Eye-white colour sampled from the white away from the pupil.
        ring = white & ~ellipse_mask((h, w), cx, cy, eye["pupil_rx"], eye["pupil_ry"], grow=8)
        white_rgb = np.median(head[ring][:, :3], axis=0)
        # Soft-edged fill so no dark fringe is left around where the pupil was.
        soft = ndimage.gaussian_filter((pupil & white).astype(np.float32), 1.2)
        soft = np.clip(soft * 1.6, 0, 1)[..., None]
        base[..., :3] = base[..., :3] * (1 - soft) + white_rgb * soft
        print("eye white", white_rgb.round())

    Image.fromarray(base.astype(np.uint8)).save(PARTS / "head_base.png", optimize=True)

    nx, ny, nr = rig["nose"]
    near = ellipse_mask((h, w), nx, ny, nr, nr)
    r, g, b = head[..., 0], head[..., 1], head[..., 2]
    salmon = (r > 200) & (g > 90) & (g < 175) & (b > 70) & (b < 160) & (r - g > 55)
    nose = ndimage.binary_closing(near & salmon, iterations=2)
    nose = ndimage.binary_fill_holes(nose)
    labels, n = ndimage.label(nose)
    if n > 1:
        sizes = ndimage.sum(nose, labels, range(1, n + 1))
        nose = labels == (int(np.argmax(sizes)) + 1)
    nose = ndimage.binary_erosion(nose, iterations=1)
    nose = ndimage.binary_dilation(nose, iterations=1)
    alpha = ndimage.gaussian_filter(nose.astype(np.float32), 0.7)
    over = head.copy()
    over[..., 3] = head[..., 3] * np.clip(alpha * 1.3, 0, 1)
    Image.fromarray(over.astype(np.uint8)).save(PARTS / "head_nose.png", optimize=True)

    # Moustache: the dark brown in its box, plus a little of its cut-paper edge.
    x0, y0, x1, y1 = rig["moustache"]
    box = np.zeros((h, w), dtype=bool)
    box[y0:y1, x0:x1] = True
    luma = head[..., :3] @ np.array([0.299, 0.587, 0.114])
    monocle = ellipse_mask((h, w), *rig["eyes"][0]["center"], 54, 54)
    dark = box & ~monocle & (luma < 115) & (head[..., 3] > 128)
    labels, n = ndimage.label(dark)
    if n:
        sizes = ndimage.sum(dark, labels, range(1, n + 1))
        dark = np.isin(labels, [i + 1 for i, s in enumerate(sizes) if s > sizes.max() * 0.1])
    tache = ndimage.binary_dilation(ndimage.binary_fill_holes(dark), iterations=2) & box & ~nose
    alpha = ndimage.gaussian_filter(tache.astype(np.float32), 0.6)
    over = head.copy()
    over[..., 3] = head[..., 3] * np.clip(alpha * 1.4, 0, 1)
    Image.fromarray(over.astype(np.uint8)).save(PARTS / "head_moustache.png", optimize=True)

    skin = np.median(head[ellipse_mask((h, w), 255, 112, 8, 6)][:, :3], axis=0)
    print("skin", skin.round(), "nose pixels", int(nose.sum()), "moustache pixels", int(tache.sum()))


if __name__ == "__main__":
    main()
