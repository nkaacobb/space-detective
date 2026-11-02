"""Generates the paper-cut desk props with Z-Image Turbo in ComfyUI and cuts them out.

    python art/make_props.py [name ...] [--seed N]

Runs workflows/paper_prop_zimage.json per prop and writes assets/props/<name>.png (transparent),
plus art/work/props_sheet.png for review.
"""

import argparse
from pathlib import Path

import numpy as np
import requests
from PIL import Image, ImageDraw
from scipy import ndimage

from comfy import COMFY, download, load_workflow, queue, wait

ROOT = Path(__file__).resolve().parent.parent
WORK = ROOT / "art" / "work" / "props"
OUT = ROOT / "assets" / "props"

STYLE = ("Paper cutout craft illustration made of layered textured cardstock, visible paper fibre texture, "
         "crisp cut edges with a thin white paper border, soft drop shadows between the paper layers, warm "
         "storybook style matching a cosy Victorian detective's desk, muted honey tan, chocolate brown, cream "
         "and gold palette. A single isolated object centred on a plain flat light grey background, "
         "no text, no letters, no table, no scene.")

PROPS = {
    "pen": ("An antique fountain pen lying diagonally, dark polished wood barrel, gold band and a gold nib.", 1024, 640),
    "teacup": ("A cream porcelain teacup with a thin gold rim on a matching saucer, filled with tea, "
               "a little curl of paper steam rising above it, seen from the side.", 1024, 1024),
    "books": ("A small stack of three old leather-bound books lying flat, burgundy, dark green and brown covers "
              "with gold bands on the spines, seen from the side.", 1024, 768),
    "papers": ("A messy stack of yellowed case-file papers and an old manila folder, a few loose notes sticking out, "
               "held together with a gold paperclip, seen at a slight angle from above.", 1024, 768),
    "compass": ("An antique brass compass rose medallion with a navy blue face and cream star points, seen from above.",
                1024, 1024),
    "fingerprint": ("A single fingerprint stamp in dark sepia ink, clear swirling ridges, flat on the paper.", 1024, 1024),
    "paperclip": ("A single gold wire paperclip, slightly bent, seen from above.", 768, 1024),
    "pushpin": ("A single round red pushpin with a glossy domed head, seen from above at a slight angle.", 1024, 1024),
}


def finish(name):
    rgb = Image.open(WORK / f"{name}_rgb.png").convert("RGB")
    mask = np.asarray(Image.open(WORK / f"{name}_mask.png").convert("L"), dtype=np.float32) / 255.0
    solid = mask > 0.5
    labels, n = ndimage.label(solid)
    if n:
        sizes = ndimage.sum(solid, labels, range(1, n + 1))
        keep = np.isin(labels, [i + 1 for i, s in enumerate(sizes) if s >= sizes.max() * 0.05])
        keep = ndimage.binary_dilation(ndimage.binary_fill_holes(keep), iterations=3)
        mask = np.where(keep, mask, 0.0)
    img = rgb.convert("RGBA")
    img.putalpha(Image.fromarray((np.clip((mask - 0.15) / 0.7, 0, 1) * 255).astype(np.uint8)))
    img = img.crop(img.getbbox())
    # Props are small in the app; half size keeps them sharp on high-DPI screens.
    img = img.resize((img.width // 2, img.height // 2), Image.LANCZOS)
    if name == "fingerprint":
        # Watermarks want just the ink ridges, not the paper card they were printed on.
        rgb_a = np.asarray(img, dtype=np.float32)
        luma = rgb_a[..., :3] @ np.array([0.299, 0.587, 0.114])
        ink = np.clip((215.0 - luma) / 150.0, 0, 1) * (rgb_a[..., 3] / 255.0)
        out = np.zeros_like(rgb_a)
        out[..., :3] = (70, 40, 24)
        out[..., 3] = ink * 255
        img = Image.fromarray(out.astype(np.uint8))
    OUT.mkdir(parents=True, exist_ok=True)
    img.save(OUT / f"{name}.png", optimize=True)
    return img


def collect(name):
    """Fetches the newest render of `name` that ComfyUI already made (if a run timed out waiting)."""
    history = requests.get(f"{COMFY}/history", params={"max_items": 400}).json()
    newest = None
    for entry in history.values():
        for out in entry.get("outputs", {}).values():
            for img in out.get("images", []):
                if img["subfolder"].endswith("props") and img["filename"].startswith(f"{name}_"):
                    kind = "rgb" if "_rgb_" in img["filename"] else "mask"
                    if newest is None or img["filename"] > newest.get(kind, {}).get("filename", ""):
                        newest = newest or {}
                        newest[kind] = img
    if not newest or len(newest) < 2:
        raise RuntimeError(f"no finished render of {name} in ComfyUI's history")
    download(newest["rgb"], WORK / f"{name}_rgb.png")
    download(newest["mask"], WORK / f"{name}_mask.png")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("names", nargs="*")
    ap.add_argument("--seed", type=int, default=41)
    ap.add_argument("--collect", action="store_true", help="use renders ComfyUI already finished")
    args = ap.parse_args()
    names = args.names or list(PROPS)
    finished = {}
    if args.collect:
        for name in names:
            collect(name)
            finished[name] = finish(name)
            print("collected", name, finished[name].size, flush=True)
    else:
        jobs = []
        for i, name in enumerate(names):
            subject, w, h = PROPS[name]
            wf = load_workflow("paper_prop_zimage", prompt=f"{subject} {STYLE}", width=w, height=h,
                               seed=args.seed + i * 7, prefix=f"space-detective/props/{name}")
            jobs.append((name, queue(wf)))
        for name, prompt_id in jobs:
            outputs = wait(prompt_id, timeout=7200)
            download(outputs["13"]["images"][0], WORK / f"{name}_rgb.png")
            download(outputs["14"]["images"][0], WORK / f"{name}_mask.png")
            finished[name] = finish(name)
            print("done", name, finished[name].size, flush=True)

    sheet = Image.new("RGBA", (300 * len(finished), 320), (24, 36, 64, 255))
    d = ImageDraw.Draw(sheet)
    for i, (name, img) in enumerate(finished.items()):
        s = min(288 / img.width, 288 / img.height)
        t = img.resize((int(img.width * s), int(img.height * s)), Image.LANCZOS)
        sheet.alpha_composite(t, (300 * i + 6, 6))
        d.text((300 * i + 6, 300), name, fill=(240, 230, 200, 255))
    sheet.convert("RGB").save(ROOT / "art" / "work" / "props_sheet.png")


if __name__ == "__main__":
    main()
