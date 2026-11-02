"""Procedural, seamlessly tiling paper textures for the app.

    python art/make_textures.py

Writes assets/textures/navy_paper.png (the desk behind everything), parchment.png (cards and the
speech bubble) and grain.png (near-white fibre grain that tints every treemap box).
"""

from pathlib import Path

import numpy as np
from PIL import Image
from scipy import ndimage

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "assets" / "textures"
rng = np.random.default_rng(7)


def fractal(size, scales, weights):
    """Tileable noise: blurred white noise with wrap-around edges, summed over a few scales."""
    total = np.zeros((size, size))
    for s, w in zip(scales, weights):
        n = ndimage.gaussian_filter(rng.standard_normal((size, size)), s, mode="wrap")
        total += w * n / (n.std() + 1e-9)
    return total / sum(weights)


def fibres(size, count, length, softness=0.6):
    """Thin wavy paper fibres, wrapped at the edges so the texture tiles."""
    canvas = np.zeros((size, size))
    for _ in range(count):
        x, y = rng.uniform(0, size, 2)
        angle = rng.uniform(0, np.pi)
        for _ in range(int(rng.uniform(0.4, 1.0) * length)):
            angle += rng.normal(0, 0.15)
            x, y = x + np.cos(angle), y + np.sin(angle)
            canvas[int(y) % size, int(x) % size] += 1.0
    return ndimage.gaussian_filter(canvas, softness, mode="wrap")


def save(name, rgb):
    OUT.mkdir(parents=True, exist_ok=True)
    Image.fromarray(np.clip(rgb, 0, 255).astype(np.uint8)).save(OUT / f"{name}.png", optimize=True)
    print("wrote", name)


def navy_paper(size=512):
    base = np.array([21, 32, 58], dtype=float)
    mottle = fractal(size, [48, 14, 3], [1.0, 0.6, 0.3])
    fib = fibres(size, 900, 40)
    img = base + mottle[..., None] * np.array([3.5, 4.5, 6.5]) + fib[..., None] * np.array([6, 8, 11])
    # A sprinkle of faint stars: this detective works in space, after all.
    stars = np.zeros((size, size))
    for _ in range(70):
        x, y = rng.integers(0, size, 2)
        stars[y, x] = rng.uniform(0.3, 1.0)
    stars = ndimage.gaussian_filter(stars, 0.7, mode="wrap") * 9
    img += stars[..., None] * np.array([55, 55, 45])
    save("navy_paper", img)


def parchment(size=512):
    base = np.array([238, 224, 192], dtype=float)
    mottle = fractal(size, [64, 20, 4], [1.0, 0.5, 0.25])
    stains = np.clip(fractal(size, [40, 12], [1.0, 0.4]) - 1.1, 0, None)
    fib = fibres(size, 700, 36)
    img = (base + mottle[..., None] * np.array([6, 7, 9])
           - stains[..., None] * np.array([22, 30, 44])
           - fib[..., None] * np.array([10, 12, 16]))
    save("parchment", img)


def grain(size=256):
    g = 244 + fractal(size, [6, 1.2], [1.0, 0.8]) * 5 - fibres(size, 260, 26, 0.6) * 11
    save("grain", np.repeat(g[..., None], 3, axis=2))


if __name__ == "__main__":
    navy_paper()
    parchment()
    grain()
