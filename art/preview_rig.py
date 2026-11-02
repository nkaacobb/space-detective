"""Renders the paper puppet from assets/detective/rig.json in each of its poses, side by side.

    python art/preview_rig.py [pose ...]

Writes art/work/rig_preview.png. The app uses the same rig maths (src/detective.rs).
"""

import json
import math
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
PARTS = ROOT / "assets" / "detective"


def mat(tx=0.0, ty=0.0, deg=0.0):
    c, s = math.cos(math.radians(deg)), math.sin(math.radians(deg))
    return np.array([[c, -s, tx], [s, c, ty], [0, 0, 1]], dtype=np.float64)


def world_transforms(rig, pose):
    parts = {p["name"]: p for p in rig["parts"]}
    cache = {}

    def world(name):
        if name in cache:
            return cache[name]
        p = parts[name]
        angle = p["angle"] + pose.get(name, 0.0)
        local = mat(*p["at"]) @ mat(deg=angle) @ mat(-p["anchor"][0], -p["anchor"][1])
        w = local if p["parent"] is None else world(p["parent"]) @ local
        cache[name] = w
        return w

    return {n: world(n) for n in parts}


def solve_arm(rig, target, glass_angle=0.0, elbow_down=True):
    """Two-bone IK: angles that put the lens centre on `target` (torso-pivot space) with the glass
    tilted `glass_angle` degrees. Mirrors Rig::solve_arm in src/detective.rs."""
    parts = {p["name"]: p for p in rig["parts"]}
    torso, sleeve, fore, glass = (parts[n] for n in ("torso", "sleeve_right", "forearm_right", "glass"))
    shoulder = np.subtract(sleeve["at"], torso["anchor"])
    upper = np.subtract(fore["at"], sleeve["anchor"])  # shoulder -> elbow, sleeve image
    lower = np.subtract(glass["at"], fore["anchor"])   # elbow -> fist, forearm image
    grip_to_lens = np.subtract(rig["lens"]["center"], glass["anchor"])
    g = math.radians(glass_angle)
    rot = np.array([[math.cos(g), -math.sin(g)], [math.sin(g), math.cos(g)]])
    fist = np.asarray(target, dtype=float) - rot @ grip_to_lens
    l1, l2 = np.linalg.norm(upper), np.linalg.norm(lower)
    to = fist - shoulder
    d = min(max(np.linalg.norm(to), abs(l1 - l2) + 1e-3), l1 + l2 - 1e-3)
    bend = math.acos((l1 * l1 + d * d - l2 * l2) / (2 * l1 * d))
    base = math.atan2(to[1], to[0])
    upper_dir = base - bend if elbow_down else base + bend
    elbow = shoulder + l1 * np.array([math.cos(upper_dir), math.sin(upper_dir)])
    lower_dir = math.atan2(*(fist - elbow)[::-1])
    sleeve_rot = math.degrees(upper_dir - math.atan2(upper[1], upper[0]))
    fore_rot = math.degrees(lower_dir - math.atan2(lower[1], lower[0])) - sleeve_rot
    glass_rot = glass_angle - sleeve_rot - fore_rot
    return {"sleeve_right": sleeve_rot, "forearm_right": fore_rot, "glass": glass_rot}


def resolve_pose(rig, pose):
    pose = dict(pose)
    if "lens_target" in pose:
        pose.update(solve_arm(rig, pose.pop("lens_target"), pose.pop("glass_angle", 0.0), pose.pop("elbow_down", True)))
    return pose


def render(rig, pose, size=900, scale=1.0, pins=True):
    pose = resolve_pose(rig, pose)
    canvas = Image.new("RGBA", (size, size), (24, 36, 64, 255))
    origin = mat(size / 2, size * 0.55) @ np.diag([scale, scale, 1.0])
    worlds = world_transforms(rig, pose)
    draw_pins = []
    for p in sorted(rig["parts"], key=lambda p: p["z"]):
        img = Image.open(PARTS / f"{p['name']}.png").convert("RGBA")
        m = origin @ worlds[p["name"]]
        inv = np.linalg.inv(m)
        layer = img.transform((size, size), Image.AFFINE, data=tuple(inv[:2].flatten()), resample=Image.BICUBIC)
        canvas.alpha_composite(layer)
        if pins and p["parent"] is not None:
            draw_pins.append(m @ np.array([p["anchor"][0], p["anchor"][1], 1.0]))
    d = ImageDraw.Draw(canvas)
    for x, y, _ in draw_pins:
        d.ellipse([x - 4, y - 4, x + 4, y + 4], fill=(212, 170, 80, 255), outline=(90, 60, 20, 255))
    lens = rig["lens"]
    m = origin @ worlds[lens["part"]]
    cx, cy, _ = m @ np.array([lens["center"][0], lens["center"][1], 1.0])
    r = lens["radius"] * scale
    d.ellipse([cx - r, cy - r, cx + r, cy + r], outline=(255, 80, 80, 255), width=2)
    return canvas


def main():
    rig = json.loads((PARTS / "rig.json").read_text())
    poses = sys.argv[1:] or list(rig["poses"])
    frames = [render(rig, rig["poses"][name]) for name in poses]
    sheet = Image.new("RGBA", (900 * len(frames), 900))
    for i, (name, f) in enumerate(zip(poses, frames)):
        ImageDraw.Draw(f).text((10, 10), name, fill=(255, 255, 255, 255))
        sheet.paste(f, (900 * i, 0))
    sheet.convert("RGB").save(ROOT / "art" / "work" / "rig_preview.png")


if __name__ == "__main__":
    main()
