"""Measures how closely each voice clip matches the approved voice (voice candidate A).

    python art/voice_match.py [folder of .flac/.wav clips]   # default: art/work/voice_raw

Compares each clip's median pitch and its average timbre (MFCCs) with the approved recording and
lists the ones that have drifted. Uses the raw takes, before the muffled effect is added.
"""

import sys
from pathlib import Path

import librosa
import numpy as np

ROOT = Path(__file__).resolve().parent.parent
# The voice that was approved (candidate A from workflows/detective_voice_candidates.json).
REFERENCE = ROOT / "art" / "voice_reference.wav"


def profile(path):
    y, sr = librosa.load(path, sr=16000, mono=True)
    y, _ = librosa.effects.trim(y, top_db=35)
    f0, voiced, _ = librosa.pyin(y, fmin=60, fmax=400, sr=sr, frame_length=1024)
    pitch = float(np.nanmedian(f0[voiced])) if np.any(voiced) else float("nan")
    mfcc = librosa.feature.mfcc(y=y, sr=sr, n_mfcc=20)[1:]
    return pitch, mfcc.mean(axis=1)


def similarity(clip_profile, ref_profile):
    """(pitch difference in semitones, relative timbre difference) between two profiles."""
    pitch, mfcc = clip_profile
    ref_pitch, ref_mfcc = ref_profile
    semitones = 12 * np.log2(pitch / ref_pitch) if pitch == pitch else float("nan")
    return semitones, float(np.linalg.norm(mfcc - ref_mfcc) / np.linalg.norm(ref_mfcc))


def main():
    folder = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "art" / "work" / "voice_raw"
    ref = profile(REFERENCE)
    rows = []
    for clip in sorted(list(folder.glob("*.flac")) + list(folder.glob("*.wav"))):
        if clip.stem.endswith("_qa"):
            continue
        rows.append((clip.stem, *similarity(profile(clip), ref)))
    print(f"approved voice: median pitch {ref[0]:.0f} Hz")
    print(f"{'clip':<24} {'pitch vs A':>11} {'timbre diff':>12}")
    for name, st, tb in sorted(rows, key=lambda r: -abs(r[1]) - r[2]):
        flag = "  <- drifted" if abs(st) > 2.0 or tb > 0.35 else ""
        print(f"{name:<24} {st:+10.1f}st {tb:12.2f}{flag}")
    st = np.array([r[1] for r in rows])
    tb = np.array([r[2] for r in rows])
    print(f"\n{len(rows)} clips: pitch spread {np.nanstd(st):.2f} semitones, mean timbre diff {np.mean(tb):.2f}, "
          f"drifted {int(np.sum((np.abs(st) > 2.0) | (tb > 0.35)))}")


if __name__ == "__main__":
    main()
