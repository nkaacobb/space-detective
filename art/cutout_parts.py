"""Step 2 of the detective art pipeline: upscale every crop 4x and mask out the paper background.

    python art/cutout_parts.py [name ...]

Runs workflows/detective_parts_cutout.json in ComfyUI for each crop in art/work/crops and saves
the upscaled image and its mask to art/work/cutouts/.
"""

import sys
from pathlib import Path

from comfy import download, load_workflow, queue, upload_image, wait

ROOT = Path(__file__).resolve().parent.parent
CROPS = ROOT / "art" / "work" / "crops"
OUT = ROOT / "art" / "work" / "cutouts"


def main():
    names = sys.argv[1:] or sorted(p.stem for p in CROPS.glob("*.png"))
    jobs = []
    for name in names:
        uploaded = upload_image(CROPS / f"{name}.png")
        wf = load_workflow("detective_parts_cutout", input=uploaded, prefix=f"space-detective/parts/{name}")
        jobs.append((name, queue(wf)))
    for name, prompt_id in jobs:
        outputs = wait(prompt_id)
        download(outputs["7"]["images"][0], OUT / f"{name}_rgb.png")
        download(outputs["8"]["images"][0], OUT / f"{name}_mask.png")
        print("done", name)


if __name__ == "__main__":
    main()
