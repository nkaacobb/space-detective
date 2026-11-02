"""Scores how much each voice clip sounds like the approved voice, using Qwen3-TTS's own speaker encoder.

Run it with ComfyUI's Python (it needs torch and the Qwen-TTS node's `qwen_tts` package):

    <ComfyUI>/python_embeded/python.exe art/speaker_match.py CLIPS_DIR \
        --node-dir <custom_nodes>/ComfyUI-Qwen-TTS --model <models>/qwen-tts/Qwen3-TTS-12Hz-1.7B-Base

Prints a similarity per clip (1.0 = identical speaker) and writes CLIPS_DIR/similarity.json.
Runs on the CPU so it doesn't fight ComfyUI for the GPU.
"""

import argparse
import json
import sys
from pathlib import Path

import librosa
import numpy as np
import torch

ROOT = Path(__file__).resolve().parent.parent
REFERENCE = ROOT / "art" / "voice_reference.wav"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("clips", type=Path)
    ap.add_argument("--node-dir", required=True, type=Path)
    ap.add_argument("--model", required=True, type=Path)
    args = ap.parse_args()

    sys.path.insert(0, str(args.node_dir))
    from qwen_tts import Qwen3TTSModel

    tts = Qwen3TTSModel.from_pretrained(str(args.model), device_map="cpu", dtype=torch.float32).model
    sr = 24000

    def embed(path):
        wav, _ = librosa.load(path, sr=sr, mono=True)
        wav, _ = librosa.effects.trim(wav, top_db=35)
        with torch.no_grad():
            e = tts.extract_speaker_embedding(audio=wav.astype(np.float32), sr=sr)
        e = e.float().cpu().numpy().ravel()
        return e / np.linalg.norm(e)

    ref = embed(REFERENCE)
    scores = {}
    for clip in sorted(list(args.clips.glob("*.flac")) + list(args.clips.glob("*.wav"))):
        if clip.stem.endswith("_qa"):
            continue
        scores[clip.stem] = float(embed(clip) @ ref)
    for name, s in sorted(scores.items(), key=lambda kv: kv[1]):
        print(f"{s:.3f}  {name}")
    values = np.array(list(scores.values()))
    print(f"\n{len(values)} clips: mean {values.mean():.3f}, lowest {values.min():.3f}, below 0.80: {int((values < 0.80).sum())}")
    (args.clips / "similarity.json").write_text(json.dumps(scores, indent=1))


if __name__ == "__main__":
    main()
