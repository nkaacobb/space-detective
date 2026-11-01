"""Draws the Space Detective icon and writes every size the Windows and macOS builds need.

    python packaging/make_icons.py

Writes assets/icon.png (1024 px), assets/icon-256.png (window icon), assets/icon.ico and assets/icon.icns.
Needs Pillow.
"""

from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter

SS = 2  # draw at twice the size, then shrink, for smooth edges
N = 1024 * SS
ASSETS = Path(__file__).resolve().parent.parent / "assets"


def s(v):
    return int(v * SS)


def gradient(size, top_left, bottom_right):
    """Diagonal gradient, lighter in the top-left, like the cushions in the app."""
    w, h = size
    small = Image.new("RGB", (2, 2))
    small.putpixel((0, 0), top_left)
    small.putpixel((1, 1), bottom_right)
    mid = tuple((a + b) // 2 for a, b in zip(top_left, bottom_right))
    small.putpixel((1, 0), mid)
    small.putpixel((0, 1), mid)
    return small.resize((w, h), Image.BILINEAR)


def mix(c, other, t):
    return tuple(round(a + (b - a) * t) for a, b in zip(c, other))


def rounded_mask(size, radius):
    mask = Image.new("L", size, 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, size[0] - 1, size[1] - 1), radius=radius, fill=255)
    return mask


def paste_tile(canvas, box, color, radius):
    x0, y0, x1, y1 = (s(v) for v in box)
    size = (x1 - x0, y1 - y0)
    fill = gradient(size, mix(color, (255, 255, 255), 0.30), mix(color, (0, 0, 0), 0.32))
    canvas.paste(fill, (x0, y0), rounded_mask(size, s(radius)))


def draw():
    img = Image.new("RGBA", (N, N), (0, 0, 0, 0))

    # Background plate on the macOS icon grid (824 px square inside 1024).
    plate_box = (100, 100, 924, 924)
    shadow = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle([s(v) for v in (100, 116, 924, 940)], radius=s(185), fill=(0, 0, 0, 110))
    img.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(s(18))))
    plate = gradient((s(824), s(824)), (44, 47, 56), (20, 21, 25))
    img.paste(plate, (s(100), s(100)), rounded_mask((s(824), s(824)), s(185)))

    # A small treemap, in the app's first six palette colours.
    tiles = [
        ((172, 172, 524, 604), (78, 121, 167)),
        ((172, 618, 524, 852), (242, 142, 43)),
        ((538, 172, 852, 424), (225, 87, 89)),
        ((538, 438, 702, 852), (118, 183, 178)),
        ((716, 438, 852, 640), (89, 161, 79)),
        ((716, 654, 852, 852), (237, 201, 72)),
    ]
    for box, color in tiles:
        paste_tile(img, box, color, 22)

    # Magnifying glass: soft shadow, glassy lens, white ring, dark handle.
    cx, cy, r, ring = 548, 520, 214, 44
    hx0, hy0, hx1, hy1 = 548 + 0.70 * r, 520 + 0.70 * r, 846, 818
    glass_shadow = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    d = ImageDraw.Draw(glass_shadow)
    off = 18
    d.ellipse(
        [s(cx - r - ring / 2), s(cy - r - ring / 2 + off), s(cx + r + ring / 2), s(cy + r + ring / 2 + off)],
        outline=(0, 0, 0, 120),
        width=s(ring),
    )
    d.line([s(hx0), s(hy0 + off), s(hx1), s(hy1 + off)], fill=(0, 0, 0, 120), width=s(84))
    img.alpha_composite(glass_shadow.filter(ImageFilter.GaussianBlur(s(16))))

    lens = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    d = ImageDraw.Draw(lens)
    d.ellipse([s(cx - r), s(cy - r), s(cx + r), s(cy + r)], fill=(255, 255, 255, 46))
    # Highlight arc across the upper-left of the glass.
    d.arc([s(cx - r + 46), s(cy - r + 46), s(cx + r - 46), s(cy + r - 46)], start=200, end=255, fill=(255, 255, 255, 150), width=s(18))
    img.alpha_composite(lens)

    d = ImageDraw.Draw(img)
    d.line([s(hx0), s(hy0), s(hx1), s(hy1)], fill=(34, 36, 42, 255), width=s(84))
    d.ellipse([s(hx1 - 42), s(hy1 - 42), s(hx1 + 42), s(hy1 + 42)], fill=(34, 36, 42, 255))
    d.line([s(hx0 + 4), s(hy0 + 4), s(hx1 - 6), s(hy1 - 6)], fill=(62, 66, 76, 255), width=s(40))
    d.ellipse([s(cx - r - ring / 2), s(cy - r - ring / 2), s(cx + r + ring / 2), s(cy + r + ring / 2)], outline=(244, 245, 248, 255), width=s(ring))

    # Keep everything inside the plate's rounded corners.
    clip = Image.new("L", (N, N), 0)
    ImageDraw.Draw(clip).rounded_rectangle([s(v) for v in plate_box], radius=s(185), fill=255)
    clip = ImageChops.lighter(clip, shadow.split()[3].filter(ImageFilter.GaussianBlur(s(18))))
    img.putalpha(ImageChops.multiply(img.split()[3], clip))

    return img.resize((1024, 1024), Image.LANCZOS)


def main():
    ASSETS.mkdir(exist_ok=True)
    icon = draw()
    icon.save(ASSETS / "icon.png")
    icon.resize((256, 256), Image.LANCZOS).save(ASSETS / "icon-256.png")
    icon.save(ASSETS / "icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    icon.save(ASSETS / "icon.icns")
    for name in ("icon.png", "icon-256.png", "icon.ico", "icon.icns"):
        print(f"{name}: {(ASSETS / name).stat().st_size:,} bytes")


if __name__ == "__main__":
    main()
