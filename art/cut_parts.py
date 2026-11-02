"""Step 1 of the detective art pipeline: cut each puppet part and pose out of the character study.

    python art/cut_parts.py

Writes art/work/crops/<name>.png (plus a contact sheet). Step 2 upscales them and removes the
paper background in ComfyUI (workflows/detective_parts_cutout.json), step 3 is art/finish_parts.py.
"""

from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
STUDY = ROOT / "Warm Storybook Detective Character Study.png"
OUT = ROOT / "art" / "work" / "crops"

# (left, top, right, bottom) in the 1672x941 character study.
PARTS = {
    # "Paper layers" panel: the exploded puppet.
    "hat": (1336, 74, 1574, 214),
    "head": (1340, 196, 1559, 346),
    "coat_left": (1343, 340, 1414, 497),
    "coat_right": (1504, 340, 1575, 497),
    "torso": (1411, 347, 1506, 470),
    "trousers": (1413, 448, 1504, 512),
    "leg_left": (1405, 514, 1452, 566),
    "leg_right": (1465, 514, 1512, 566),
    "boot_left": (1380, 562, 1447, 612),
    "boot_right": (1470, 562, 1537, 612),
    "sleeve_left": (1318, 390, 1385, 472),
    "sleeve_right": (1533, 390, 1597, 472),
    "forearm_left": (1318, 480, 1407, 556),
    "forearm_right": (1510, 480, 1592, 556),
    "glass": (1585, 430, 1662, 576),
    # Main turnaround views.
    "view_front": (28, 138, 300, 560),
    "view_hero": (330, 138, 712, 560),
}


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    study = Image.open(STUDY).convert("RGB")
    crops = {}
    for name, box in PARTS.items():
        crop = study.crop(box)
        crop.save(OUT / f"{name}.png")
        crops[name] = crop

    # Contact sheet at 2x so the crops can be checked by eye.
    cell = 260
    cols = 6
    rows = (len(crops) + cols - 1) // cols
    sheet = Image.new("RGB", (cols * cell, rows * (cell + 18)), (60, 60, 70))
    draw = ImageDraw.Draw(sheet)
    for i, (name, crop) in enumerate(crops.items()):
        scale = min((cell - 10) / crop.width, (cell - 10) / crop.height, 2.0)
        thumb = crop.resize((int(crop.width * scale), int(crop.height * scale)), Image.LANCZOS)
        x, y = (i % cols) * cell, (i // cols) * (cell + 18)
        sheet.paste(thumb, (x + 5, y + 5))
        draw.text((x + 5, y + cell), name, fill=(255, 255, 255))
    sheet.save(OUT.parent / "crops_sheet.png")
    print(f"{len(crops)} crops written to {OUT}")


if __name__ == "__main__":
    main()
