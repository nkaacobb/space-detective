"""Small helpers for running the saved workflows in workflows/ against the local ComfyUI."""

import json
import time
from pathlib import Path

import requests

COMFY = "http://127.0.0.1:8188"
ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS = ROOT / "workflows"


def load_workflow(name, **placeholders):
    """Reads workflows/<name>.json and swaps __KEY__ placeholders for values."""
    text = (WORKFLOWS / f"{name}.json").read_text(encoding="utf-8")
    for key, value in placeholders.items():
        text = text.replace(f"__{key.upper()}__", json.dumps(value)[1:-1] if isinstance(value, str) else str(value))
    return json.loads(text)


def upload_image(path):
    with open(path, "rb") as f:
        r = requests.post(f"{COMFY}/upload/image", files={"image": (Path(path).name, f, "image/png")},
                          data={"overwrite": "true", "subfolder": "space-detective"})
    r.raise_for_status()
    info = r.json()
    return f"{info['subfolder']}/{info['name']}" if info.get("subfolder") else info["name"]


def queue(workflow):
    r = requests.post(f"{COMFY}/prompt", json={"prompt": workflow, "client_id": "space-detective"})
    if r.status_code != 200:
        raise RuntimeError(r.text)
    return r.json()["prompt_id"]


def wait(prompt_id, timeout=1800):
    start = time.time()
    while time.time() - start < timeout:
        h = requests.get(f"{COMFY}/history/{prompt_id}").json()
        if prompt_id in h:
            entry = h[prompt_id]
            if entry["status"].get("status_str") == "error":
                raise RuntimeError(json.dumps(entry["status"], indent=1)[:2000])
            return entry["outputs"]
        time.sleep(1.0)
    raise TimeoutError(prompt_id)


def download(file_info, dest):
    r = requests.get(f"{COMFY}/view", params={
        "filename": file_info["filename"], "subfolder": file_info["subfolder"], "type": file_info["type"]})
    r.raise_for_status()
    Path(dest).parent.mkdir(parents=True, exist_ok=True)
    Path(dest).write_bytes(r.content)
    return dest
