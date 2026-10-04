"""Hum: a neural pitch tracker for singing, humming and playing.

Input: a 64 ms frame (1024 samples at 16 kHz) as a log-frequency spectrum,
216 bins, 3 per semitone from 40 Hz (`features.hum_features`).
Output: 146 classes -- 145 pitches a third of a semitone apart from C2
(MIDI 36) to C6 (MIDI 84), and "no pitch".

The training frames are synthesized: harmonic tones with random spectral
tilt, vowel formants, odd-harmonic (clarinet-like) and missing-fundamental
spectra, vibrato, glides, onsets mid-frame and breath noise at random
signal-to-noise ratios; and unpitched frames of coloured noise, fricatives
and clicks. Targets are soft (a Gaussian a third of a semitone wide
around the true pitch), which lets the app read pitch between bins.
"""

import sys
import numpy as np
import torch
import torch.nn as nn

import features as F
import pmxn

LO = 36
N_PITCH = 145
UNVOICED = 145
N_OUT = 146

SPEC = [
    ("conv", 1, 16, 7, 1, 3, True),
    ("pool", 2),
    ("conv", 16, 32, 5, 1, 2, True),
    ("pool", 2),
    ("conv", 32, 32, 5, 1, 2, True),
    ("flatten",),
    ("dense", 32 * 54, 128, True),
    ("dense", 128, N_OUT, False),
]


def coloured_noise(rng, n, length):
    w = rng.normal(size=(n, length)).astype(np.float32)
    # Random one-pole colouring: white, pink-ish or brown-ish.
    a = rng.uniform(-0.2, 0.97, size=(n, 1)).astype(np.float32)
    out = np.empty_like(w)
    acc = np.zeros((n,), dtype=np.float32)
    for t in range(length):
        acc = a[:, 0] * acc + w[:, t]
        out[:, t] = acc
    out /= np.maximum(out.std(axis=1, keepdims=True), 1e-6)
    return out


def formant_env(rng, freqs):
    """A random vowel-like envelope over `freqs` [n, h]."""
    n = freqs.shape[0]
    env = np.full(freqs.shape, 0.04, dtype=np.float32)
    for lo, hi in ((250, 900), (700, 2500), (2200, 3300)):
        fc = rng.uniform(lo, hi, size=(n, 1))
        bw = rng.uniform(60, 220, size=(n, 1))
        g = rng.uniform(0.3, 1.0, size=(n, 1))
        env += (g / (1 + ((freqs - fc) / bw) ** 2)).astype(np.float32)
    return env


def make(rng, n):
    t = (np.arange(F.HUM_FRAME) / F.SR).astype(np.float32)
    voiced = rng.random(n) < 0.85
    midi = rng.uniform(LO - 0.5, LO + 48.4, size=n)
    f0 = 440.0 * 2 ** ((midi - 69) / 12)
    x = np.zeros((n, F.HUM_FRAME), dtype=np.float32)
    max_h = int(2800 / 60) + 1
    h = np.arange(1, max_h + 1)
    tilt = rng.uniform(0.3, 2.4, size=(n, 1))
    amps = h[None, :] ** (-tilt) * np.exp(rng.normal(0, 0.45, size=(n, max_h)))
    freqs = f0[:, None] * h[None, :]
    use_formant = rng.random(n) < 0.7
    amps = np.where(use_formant[:, None], amps * formant_env(rng, freqs), amps)
    odd = rng.random(n) < 0.15
    amps = np.where(odd[:, None] & (h[None, :] % 2 == 0), amps * 0.08, amps)
    missing = rng.random(n) < 0.08
    amps[:, 0] = np.where(missing, amps[:, 0] * 0.05, amps[:, 0])
    amps = np.where(freqs < 2800, amps, 0.0)
    vib_depth = rng.uniform(0, 0.012, size=(n, 1)) * (rng.random((n, 1)) < 0.6)
    vib_rate = rng.uniform(4, 7, size=(n, 1))
    vib_phase = rng.uniform(0, 2 * np.pi, size=(n, 1))
    glide = rng.normal(0, 0.15, size=(n, 1)) * (rng.random((n, 1)) < 0.2)  # semitones across the frame
    tc = t[None, :] - t[F.HUM_FRAME // 2]
    ratio = 2 ** (glide * tc / t[-1] / 12) * (1 + vib_depth * np.sin(2 * np.pi * vib_rate * tc + vib_phase))
    # Instantaneous phase of the fundamental, so vibrato is smooth.
    inst = np.cumsum(f0[:, None] * ratio, axis=1) / F.SR
    phase0 = rng.uniform(0, 1, size=(n, max_h))
    for k in range(max_h):
        live = amps[:, k] > 0
        if not live.any():
            continue
        x[live] += (amps[live, k:k + 1] * np.sin(2 * np.pi * ((k + 1) * inst[live] + phase0[live, k:k + 1]))).astype(np.float32)
    x /= np.maximum(np.abs(x).max(axis=1, keepdims=True), 1e-6)
    # Some frames hold the start of a note.
    onset = rng.random(n) < 0.15
    start = rng.integers(0, F.HUM_FRAME // 2, size=n)
    ramp = np.clip((np.arange(F.HUM_FRAME)[None, :] - start[:, None]) / 160.0, 0, 1).astype(np.float32)
    x = np.where(onset[:, None], x * ramp, x)
    snr_db = rng.uniform(0, 40, size=(n, 1))
    noise = coloured_noise(rng, n, F.HUM_FRAME)
    sig_rms = np.maximum(np.sqrt((x ** 2).mean(axis=1, keepdims=True)), 1e-6)
    x = x + noise * sig_rms * 10 ** (-snr_db / 20)
    # Unpitched frames: noise, fricatives, clicks.
    un = ~voiced
    k = un.sum()
    if k:
        z = coloured_noise(rng, k, F.HUM_FRAME)
        clicks = rng.random(k) < 0.2
        pos = rng.integers(0, F.HUM_FRAME, size=k)
        z = np.where(clicks[:, None], z * 0.05, z)
        z[np.arange(k)[clicks], pos[clicks]] += 3.0
        x[un] = z
    x *= rng.uniform(0.01, 1.0, size=(n, 1)).astype(np.float32)
    target = np.zeros((n, N_OUT), dtype=np.float32)
    pos = np.clip((midi - LO) * 3, 0, N_PITCH - 1)
    grid = np.arange(N_PITCH)[None, :]
    g = np.exp(-0.5 * ((grid - pos[:, None]) / 1.0) ** 2)
    g /= g.sum(axis=1, keepdims=True)
    target[:, :N_PITCH] = np.where(voiced[:, None], g, 0)
    target[un, UNVOICED] = 1.0
    feats = F.hum_features(x)[:, None, :]
    return feats.astype(np.float32), target, np.where(voiced, midi, -1.0), x


def soft_ce(out, target):
    return -(target * torch.log_softmax(out, dim=1)).sum(dim=1).mean()


def decode(logits):
    """Pitch in MIDI from the probabilities near the peak, or -1."""
    p = np.exp(logits - logits.max(axis=1, keepdims=True))
    p /= p.sum(axis=1, keepdims=True)
    best = p[:, :N_PITCH].argmax(axis=1)
    voiced = p[:, UNVOICED] < 0.5
    out = np.full(len(p), -1.0)
    for i in range(len(p)):
        lo, hi = max(0, best[i] - 2), min(N_PITCH, best[i] + 3)
        w = p[i, lo:hi]
        pos = (w * np.arange(lo, hi)).sum() / max(w.sum(), 1e-9)
        out[i] = LO + pos / 3.0 if voiced[i] else -1.0
    return out


def report(midi_true, midi_est):
    v = midi_true >= 0
    vd = (midi_est >= 0) == v
    pv = v & (midi_est >= 0)
    err = np.abs(midi_est[pv] - midi_true[pv]) * 100
    within = (err < 50).mean() if pv.any() else 0
    return f"voicing {vd.mean() * 100:.1f}%  within 50c {within * 100:.1f}%  median err {np.median(err) if pv.any() else 0:.0f}c"


def main():
    rng = np.random.default_rng(1)
    torch.manual_seed(1)
    n = int(sys.argv[1]) if len(sys.argv) > 1 else 120_000
    xs, ys, ms = [], [], []
    for i in range(0, n, 4000):
        a, b, m, _ = make(rng, min(4000, n - i))
        xs.append(a)
        ys.append(b)
        ms.append(m)
        print("made", i + len(a), flush=True)
    x, y, m = np.concatenate(xs), np.concatenate(ys), np.concatenate(ms)
    vx, vy, vm, _ = make(np.random.default_rng(99), 4000)
    net = pmxn.Net(SPEC, (1, F.HUM_BINS))

    def val(net):
        with torch.no_grad():
            out = net(torch.tensor(vx)).numpy()
        return report(vm, decode(out))

    pmxn.train(net, x, y, epochs=int(sys.argv[2]) if len(sys.argv) > 2 else 12, lr=3e-3, loss_fn=soft_ce, val=val)
    q = pmxn.quantise(net, x[np.random.default_rng(5).choice(len(x), 2000, replace=False)])
    sub = slice(0, 1500)
    int8 = np.stack([pmxn.dequant(q, pmxn.run_int8(q, v)) for v in vx[sub]])
    print("float:", val(net))
    print("int8: ", report(vm[sub], decode(int8)))
    # Test vectors: a sung A3 and a noise frame from the validation set.
    tests = [vx[i] for i in (0, 1, 2)]
    macs = pmxn.export(net, q, "hum", "../../assets/npu", tests, meta=dict(lo=LO, per_semitone=3, n_pitch=N_PITCH, unvoiced=UNVOICED))
    # Feature cross-check: a known frame and its features.
    tt = np.arange(F.HUM_FRAME) / F.SR
    frame = (0.5 * np.sin(2 * np.pi * 220 * tt) + 0.25 * np.sin(2 * np.pi * 440 * tt + 1) + 0.1 * np.sin(2 * np.pi * 660 * tt + 2)).astype(np.float32)
    import json
    json.dump(dict(frame=[float(v) for v in frame], features=[float(v) for v in F.hum_features(frame[None])[0]]), open("../../assets/npu/hum.features.json", "w"))
    print("MACs per frame", macs)


if __name__ == "__main__":
    main()
