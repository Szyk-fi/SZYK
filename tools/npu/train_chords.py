"""Band Mate: chord recognition from audio.

Input: a 256 ms frame (4096 samples at 16 kHz) as the energy in each
semitone from C2 to B7, 72 bins (`features.chord_features`). Output: 25
classes -- 12 major chords, 12 minor chords and "no chord".

The training frames are synthesized chords: every root, major and minor
triads plus the sevenths and added notes players use (7 and maj7 count as
major, m7 as minor, a bare fifth as major), in guitar-shaped and piano
voicings and inversions, each note with its own harmonic timbre, level and
slight detuning, strummed or struck at different points in the frame,
over noise. "No chord" is noise, silence, single notes and percussion-like
bursts.
"""

import sys
import numpy as np
import torch

import features as F
import pmxn

NAMES = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"]
CLASSES = [n for n in NAMES] + [n + "m" for n in NAMES] + ["N"]
SPEC = [
    ("conv", 1, 16, 13, 1, 6, True),
    ("pool", 2),
    ("conv", 16, 32, 7, 1, 3, True),
    ("pool", 2),
    ("flatten",),
    ("dense", 32 * 18, 64, True),
    ("dense", 64, 25, False),
]
T = np.arange(F.CHORD_FRAME) / F.SR
QUALITIES = [
    # (intervals, minor?)
    ([0, 4, 7], False), ([0, 3, 7], True), ([0, 4, 7, 10], False), ([0, 4, 7, 11], False), ([0, 3, 7, 10], True),
    ([0, 4, 7, 14], False), ([0, 3, 7, 14], True), ([0, 7], False), ([0, 4, 7, 9], False),
]


def tone(rng, midi, n_samples):
    f0 = 440 * 2 ** ((midi - 69 + rng.normal(0, 0.12)) / 12)
    tilt = rng.uniform(0.6, 2.2)
    x = np.zeros(n_samples)
    for h in range(1, 30):
        if f0 * h > 7500:
            break
        x += h ** -tilt * np.exp(rng.normal(0, 0.4)) * np.sin(2 * np.pi * f0 * h * T[:n_samples] + rng.uniform(0, 6.3))
    return x


def chord(rng, root, quality):
    intervals, _ = quality
    style = rng.integers(0, 3)
    notes = []
    if style == 0:  # guitar-like: root low, intervals spread over two octaves
        base = 40 + (root - 40) % 12
        notes = [base] + [base + i + 12 * rng.integers(0, 2) for i in intervals[1:]] + [base + 12]
        if rng.random() < 0.5:
            notes.append(base + intervals[1] + 12)
    elif style == 1:  # piano: bass root plus a close voicing above
        bass = 36 + (root - 36) % 12
        top = 55 + rng.integers(0, 12)
        notes = [bass] + [top + ((root + i - top) % 12) for i in intervals]
    else:  # an inversion: the lowest note is the third or fifth
        bass_iv = intervals[rng.integers(0, len(intervals))]
        bass = 40 + (root + bass_iv - 40) % 12
        notes = [bass] + [52 + ((root + i - 52) % 12) for i in intervals]
    x = np.zeros(F.CHORD_FRAME)
    strum = rng.random() < 0.5
    for j, m in enumerate(notes):
        e = tone(rng, m, F.CHORD_FRAME) * rng.uniform(0.3, 1.0)
        if strum:
            # The frame may catch the strum: each string starts a little later.
            start = int(max(0, rng.uniform(-0.2, 0.5) * F.CHORD_FRAME + j * rng.uniform(0, 300)))
            e[:start] = 0
            e[start:] *= np.exp(-(T[: F.CHORD_FRAME - start]) / rng.uniform(0.3, 2.0))
        x += e
    return x


def make(rng, n):
    xs, ys = [], []
    for _ in range(n):
        if rng.random() < 0.12:
            kind = rng.integers(0, 4)
            if kind == 0:
                x = rng.normal(size=F.CHORD_FRAME)
            elif kind == 1:
                x = tone(rng, rng.integers(40, 84), F.CHORD_FRAME)
            elif kind == 2:
                x = rng.normal(size=F.CHORD_FRAME) * np.exp(-T / rng.uniform(0.02, 0.2))
            else:
                x = tone(rng, rng.integers(40, 70), F.CHORD_FRAME) + tone(rng, rng.integers(40, 84), F.CHORD_FRAME) * 0.5
                # two unrelated notes: a dissonant interval
            y = 24
        else:
            root = rng.integers(0, 12)
            qi = rng.integers(0, len(QUALITIES))
            q = QUALITIES[qi]
            x = chord(rng, root, q)
            y = root + (12 if q[1] else 0)
        x = x / max(np.abs(x).max(), 1e-9)
        x += rng.normal(size=x.shape) * 10 ** (-rng.uniform(15, 50) / 20)
        xs.append(x * rng.uniform(0.02, 1.0))
        ys.append(y)
    return F.chord_features(np.array(xs, np.float32))[:, None, :], np.array(ys)


def main():
    rng = np.random.default_rng(5)
    torch.manual_seed(5)
    n = int(sys.argv[1]) if len(sys.argv) > 1 else 60000
    xs, ys = [], []
    for i in range(0, n, 2000):
        a, b = make(rng, 2000)
        xs.append(a)
        ys.append(b)
        print("made", i + 2000, flush=True)
    x, y = np.concatenate(xs), np.concatenate(ys)
    vx, vy = make(np.random.default_rng(10), 3000)
    net = pmxn.Net(SPEC, (1, F.CHORD_BINS))

    def val(net):
        with torch.no_grad():
            p = net(torch.tensor(vx)).argmax(1).numpy()
        root_ok = ((p % 12) == (vy % 12)) | (vy == 24)
        return f"acc {(p == vy).mean() * 100:.1f}%  root {root_ok[vy < 24].mean() * 100:.1f}%"

    pmxn.train(net, x, y, epochs=int(sys.argv[2]) if len(sys.argv) > 2 else 20, lr=3e-3, val=val)
    q = pmxn.quantise(net, x[np.random.default_rng(5).choice(len(x), 2000, replace=False)])
    p8 = np.array([pmxn.run_int8(q, v).argmax() for v in vx[:1500]])
    print(f"int8 acc {(p8 == vy[:1500]).mean() * 100:.1f}%")
    macs = pmxn.export(net, q, "chords", "../../assets/npu", [vx[0], vx[1], vx[2]], meta=dict(classes=CLASSES, lo=F.CHORD_LO))
    import json
    frame = (np.sin(2 * np.pi * 261.63 * T) + np.sin(2 * np.pi * 329.63 * T) + np.sin(2 * np.pi * 392.0 * T)).astype(np.float32) / 3
    json.dump(dict(frame=[float(v) for v in frame], features=[float(v) for v in F.chord_features(frame[None])[0]]), open("../../assets/npu/chords.features.json", "w"))
    print("MACs", macs)


if __name__ == "__main__":
    main()
