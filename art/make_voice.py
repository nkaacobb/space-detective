"""Generates the detective's voice lines with Qwen3-TTS in ComfyUI, cloned from the chosen voice.

    python art/make_voice.py                  # every line that isn't locked yet
    python art/make_voice.py id1 id2          # just these lines (if they aren't locked)
    python art/make_voice.py --use 1103 id1   # keep the take already recorded with seed 1103
    python art/make_voice.py --redo --seed 1104 --more id1   # re-record a locked line: three more takes

Each line is recorded several times with a fixed set of seeds (1101, 1102, 1103 by default), all
cloned from voice candidate A. Every take is transcribed by the local whisper.cpp server and
compared with the approved voice (pitch and timbre, see voice_match.py); the take closest to the
approved voice that says the right words is kept. It's trimmed, loudness-matched and saved as
assets/voice/<id>.ogg.

The seed of every kept take is locked in art/voice_seeds.json. Locked lines are never re-recorded
or swapped for another take unless --redo or --use says so, because the scores can prefer a take
that sounds wrong to a listener.
"""

import argparse
import copy
import difflib
import json
import re
import shutil
import subprocess
from pathlib import Path

import requests

from comfy import download, load_workflow, queue, upload_image, wait
from voice_match import REFERENCE, profile, similarity

ROOT = Path(__file__).resolve().parent.parent
VOICE = ROOT / "assets" / "voice"
RAW = ROOT / "art" / "work" / "voice_raw"
SEEDS = ROOT / "art" / "voice_seeds.json"
REF_TEXT = ("Aha! Well... I believe what we have here is a very large file! Good heavens, it's positively "
            "enormous. Elementary, my dear user. Quite elementary.")
WHISPER = "http://127.0.0.1:5150/inference"
# Clone nodes per ComfyUI prompt.
BATCH = 10


def lines():
    return json.loads((VOICE / "lines.json").read_text(encoding="utf-8"))


def build_batch(jobs, ref_name, sampling):
    """One prompt that clones the voice once and records each (line, seed) take."""
    template = load_workflow("detective_voice_line", ref_audio=ref_name, ref_text=REF_TEXT, text="", prefix="")
    wf = {"1": template["1"], "2": template["2"]}
    for i, (line, seed) in enumerate(jobs):
        clone = copy.deepcopy(template["10"])
        clone["inputs"]["target_text"] = line["text"]
        clone["inputs"]["seed"] = seed
        clone["inputs"].update(sampling)
        save = copy.deepcopy(template["11"])
        save["inputs"]["audio"] = [f"c{i}", 0]
        save["inputs"]["filename_prefix"] = f"space-detective/lines/{line['id']}_s{seed}"
        wf[f"c{i}"] = clone
        wf[f"s{i}"] = save
    return wf


def postprocess(raw, out, fx=None):
    af = ("silenceremove=start_periods=1:start_threshold=-50dB:start_silence=0.08,areverse,"
          "silenceremove=start_periods=1:start_threshold=-50dB:start_silence=0.12,areverse,"
          "loudnorm=I=-16:TP=-1.5:LRA=11")
    if fx == "muffled":
        # Shouting from inside a cardboard box: no treble, a little boxy echo.
        af += ",lowpass=f=1100,aecho=0.8:0.5:35|70:0.35|0.2,volume=1.4"
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", str(raw), "-af", af, "-ac", "1", "-ar", "32000",
                    "-c:a", "libvorbis", "-q:a", "4", str(out)], check=True)


NUMBERS = {"10,000": "ten thousand", "1,000": "one thousand", "100": "a hundred", "11": "eleven"}


def words(text):
    text = text.lower().replace("’", "'")
    for digits, spelled in NUMBERS.items():
        text = re.sub(rf"\b{re.escape(digits)}\b", spelled, text)
    return re.sub(r"[^a-z0-9' ]+", " ", text).split()


def transcribe(clip):
    wav = clip.with_name(clip.stem + "_qa.wav")
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", str(clip), "-ar", "16000", "-ac", "1", str(wav)], check=True)
    with open(wav, "rb") as f:
        r = requests.post(WHISPER, files={"file": (wav.name, f, "audio/wav")},
                          data={"temperature": "0", "response_format": "json"}, timeout=120)
    wav.unlink(missing_ok=True)
    return " ".join(r.json().get("text", "").split())


def duration(path):
    out = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", str(path)],
                         capture_output=True, text=True, check=True)
    return float(out.stdout.strip())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("ids", nargs="*")
    ap.add_argument("--missing", action="store_true", help="only lines that don't have a clip yet")
    ap.add_argument("--seed", type=int, default=1101, help="first of the fixed seeds")
    ap.add_argument("--takes", type=int, default=3, help="takes per line, with seeds seed, seed+1, ...")
    ap.add_argument("--temperature", type=float, help="override the workflow's sampling temperature")
    ap.add_argument("--top-k", type=int, help="override the workflow's top-k")
    ap.add_argument("--more", action="store_true",
                    help="also choose from takes already on disk with other seeds (only if the text hasn't changed)")
    ap.add_argument("--redo", action="store_true", help="re-record lines even though their seed is locked")
    ap.add_argument("--use", type=int, metavar="SEED", help="keep the take already recorded with this seed")
    args = ap.parse_args()
    sampling = {k: v for k, v in (("temperature", args.temperature), ("top_k", args.top_k)) if v is not None}
    seeds = [args.seed + k for k in range(args.takes)]
    kept = json.loads(SEEDS.read_text()) if SEEDS.exists() else {}

    todo = [l for l in lines() if not args.ids or l["id"] in args.ids]
    if args.missing:
        todo = [l for l in todo if not (VOICE / f"{l['id']}.ogg").exists()]
    takes_dir = RAW / "takes"
    takes_dir.mkdir(parents=True, exist_ok=True)

    if args.use is not None:
        if not args.ids:
            ap.error("--use needs the ids of the lines to keep it for")
        for line in todo:
            keep(line, args.use, takes_dir / f"{line['id']}_s{args.use}.flac")
            kept[line["id"]] = args.use
            print(f"kept {line['id']} seed {args.use}")
        save(kept)
        return
    if not args.redo:
        locked = [l["id"] for l in todo if l["id"] in kept]
        if locked:
            print("locked, pass --redo to re-record:", " ".join(locked))
        todo = [l for l in todo if l["id"] not in kept]
    if not todo:
        return
    ref = profile(REFERENCE)

    ref_name = upload_image(REFERENCE)
    jobs = [(line, seed) for line in todo for seed in seeds]
    prompts = []
    for start in range(0, len(jobs), BATCH):
        batch = jobs[start:start + BATCH]
        wf = build_batch(batch, ref_name, sampling)
        if start == 0:
            (ROOT / "workflows" / "detective_voice_lines_batch.json").write_text(json.dumps(wf, indent=1))
        prompts.append((batch, queue(wf)))

    takes = {}
    clean = {}
    for batch, prompt_id in prompts:
        outputs = wait(prompt_id, timeout=7200)
        for i, (line, seed) in enumerate(batch):
            path = download(outputs[f"s{i}"]["audio"][0], takes_dir / f"{line['id']}_s{seed}.flac")
            takes.setdefault(line["id"], []).append((seed, path))
        # Pick each line as soon as all its takes are in.
        for line in todo:
            if line["id"] not in clean and len(takes.get(line["id"], [])) == len(seeds):
                choices = takes[line["id"]]
                if args.more:
                    for old in takes_dir.glob(f"{line['id']}_s*.flac"):
                        seed = int(old.stem.rsplit("_s", 1)[1])
                        if seed not in seeds:
                            choices.append((seed, old))
                clean[line["id"]], kept[line["id"]] = pick(line, choices, ref)
                save(kept)

    bad = [id for id, ok in clean.items() if not ok]
    print("still no clean take:", " ".join(bad) if bad else "none")
    save(kept)


def save(kept):
    """Locks the kept seeds and records each clip's length."""
    order = [l["id"] for l in lines()]
    SEEDS.write_text(json.dumps({id: kept[id] for id in order if id in kept}, indent=1))
    # The app times each thought bubble's typewriter text to its clip.
    durations = {id: round(duration(VOICE / f"{id}.ogg"), 2) for id in order if (VOICE / f"{id}.ogg").exists()}
    (VOICE / "durations.json").write_text(json.dumps(durations, indent=1))


def keep(line, seed, take):
    shutil.copyfile(take, RAW / f"{line['id']}.flac")
    postprocess(take, VOICE / f"{line['id']}.ogg", line.get("fx"))


def pick(line, takes, ref):
    """Keeps the take that says the right words and sounds most like the approved voice.
    Returns whether it said the right words, and its seed."""
    scored = []
    for seed, path in takes:
        heard = transcribe(path)
        words_ok = difflib.SequenceMatcher(None, words(line["text"]), words(heard)).ratio()
        semitones, timbre = similarity(profile(path), ref)
        # Expression moves pitch a little; only penalise drift beyond a couple of semitones.
        drift = max(0.0, abs(semitones) - 2.0) + 2.0 * timbre
        scored.append((words_ok >= 0.85, -drift, words_ok, seed, path, semitones, timbre))
    scored.sort(key=lambda s: (s[0], s[1], s[2]), reverse=True)
    ok, neg_drift, words_ok, seed, path, semitones, timbre = scored[0]
    keep(line, seed, path)
    print(f"{'OK ' if ok else 'BAD'} {line['id']:<22} seed {seed}  pitch {semitones:+.1f}st  timbre {timbre:.2f}  "
          f"words {words_ok:.2f}", flush=True)
    return ok, seed


if __name__ == "__main__":
    main()
