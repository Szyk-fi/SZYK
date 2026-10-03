"""Mouth Drums: a sound-embedding network for teaching Portamax your own
sounds.

Input: the first ~80 ms of a sound (from just before its onset) as 32 mel
bands x 12 frames (`features.mouth_features`). Output: a 64-number
embedding. On the device the user records a few examples of each sound;
their average embedding is that sound's prototype, and a new sound goes to
the nearest prototype (a prototypical network). Learning a new sound is
just an average, so it happens instantly, on the device.

The network is trained to tell apart 32 families of synthesized mouth and
body percussion -- kicks ("b", "boom"), snares ("pf", "k-sh"), hats
("ts"), plosives, fricatives, vowels, hums, whistles, clicks, claps,
snaps and knocks -- each with wide random variation in pitch, length,
tone and noise. Only the embedding layer ships; the 32-way classifier on
top of it exists only for training.
"""

import sys
import numpy as np
import torch

import features as F
import pmxn

L = F.MOUTH_SAMPLES
ONSET = F.MOUTH_PRE * F.MOUTH_HOP
EMBED = 64

SPEC = [
    ("conv", F.MOUTH_MELS, 48, 3, 1, 1, True),
    ("conv", 48, 64, 3, 2, 1, True),
    ("conv", 64, 64, 3, 2, 1, True),
    ("flatten",),
    ("dense", 64 * 3, EMBED, True),
]
HEAD = ("dense", EMBED, 32, False)

T = np.arange(L) / F.SR


def env(rng, n, attack, decay, start):
    """Attack/decay envelope from `start` (samples)."""
    a = attack[:, None]
    d = decay[:, None]
    tt = T[None, :] - start[:, None] / F.SR
    e = np.where(tt < 0, 0.0, np.where(tt < a, tt / np.maximum(a, 1e-4), np.exp(-(tt - a) / np.maximum(d, 1e-4))))
    return e


def shaped_noise(rng, n, fc, width, kind):
    """Noise filtered in the frequency domain: band-pass around fc
    (log-Gaussian, width in octaves), high-pass above fc or low-pass
    below it."""
    w = rng.normal(size=(n, L))
    s = np.fft.rfft(w, axis=1)
    f = np.fft.rfftfreq(L, 1 / F.SR)[None, :]
    lf = np.log2(np.maximum(f, 1) / fc[:, None])
    if kind == "bp":
        g = np.exp(-0.5 * (lf / width[:, None]) ** 2)
    elif kind == "hp":
        g = 1 / (1 + np.exp(-lf * 6))
    else:
        g = 1 / (1 + np.exp(lf * 6))
    out = np.fft.irfft(s * g, n=L, axis=1)
    return out / np.maximum(out.std(axis=1, keepdims=True), 1e-9)


def sweep(rng, n, f_start, f_end, tau, start):
    tt = np.maximum(T[None, :] - start[:, None] / F.SR, 0)
    f = f_end[:, None] + (f_start - f_end)[:, None] * np.exp(-tt / tau[:, None])
    ph = np.cumsum(f, axis=1) / F.SR
    return np.sin(2 * np.pi * ph)


def voice(rng, n, f0, formants):
    """A buzzy voice through formants (additive, harmonics to 7 kHz)."""
    out = np.zeros((n, L))
    vib = 1 + 0.01 * np.sin(2 * np.pi * rng.uniform(4, 7, (n, 1)) * T[None, :])
    ph = np.cumsum(f0[:, None] * vib, axis=1) / F.SR
    for h in range(1, 60):
        fh = f0 * h
        amp = np.zeros(n) + 0.03
        for (fc, bw) in formants:
            amp += 1 / (1 + ((fh - fc) / bw) ** 2)
        amp = np.where(fh < 7000, amp / h ** 0.6, 0)
        if not amp.any():
            break
        out += amp[:, None] * np.sin(2 * np.pi * h * ph)
    return out / np.maximum(np.abs(out).max(axis=1, keepdims=True), 1e-9)


def u(rng, n, a, b):
    return rng.uniform(a, b, size=n)


def vowel(rng, n, f1, f2):
    return [(u(rng, n, f1 * 0.85, f1 * 1.15), u(rng, n, 80, 160)), (u(rng, n, f2 * 0.85, f2 * 1.15), u(rng, n, 100, 220)), (u(rng, n, 2500, 3200), u(rng, n, 150, 300))]


def family(k, rng, n):
    s = ONSET + rng.integers(-40, 41, size=n)
    E = lambda a, d, st=None: env(rng, n, a, d, s if st is None else st)
    if k == 0:  # kick "b"
        return sweep(rng, n, u(rng, n, 120, 220), u(rng, n, 45, 70), u(rng, n, 0.01, 0.04), s) * E(u(rng, n, 0.001, 0.005), u(rng, n, 0.05, 0.2)) + 0.3 * shaped_noise(rng, n, u(rng, n, 300, 900), None, "lp") * E(np.full(n, 0.001), u(rng, n, 0.005, 0.015))
    if k == 1:  # snare "pf"
        return shaped_noise(rng, n, u(rng, n, 1500, 4000), u(rng, n, 0.6, 1.2), "bp") * E(u(rng, n, 0.001, 0.004), u(rng, n, 0.05, 0.15)) + 0.4 * sweep(rng, n, u(rng, n, 200, 300), u(rng, n, 150, 200), np.full(n, 0.02), s) * E(np.full(n, 0.001), u(rng, n, 0.02, 0.05))
    if k == 2:  # snare "k-sh"
        return 0.8 * shaped_noise(rng, n, u(rng, n, 1500, 3000), u(rng, n, 0.3, 0.6), "bp") * E(np.full(n, 0.0005), np.full(n, 0.006)) + shaped_noise(rng, n, u(rng, n, 2500, 4000), None, "hp") * E(u(rng, n, 0.005, 0.02), u(rng, n, 0.06, 0.15))
    if k == 3:  # hat "ts"
        return shaped_noise(rng, n, u(rng, n, 5000, 7500), None, "hp") * E(u(rng, n, 0.0005, 0.003), u(rng, n, 0.01, 0.04))
    if k == 4:  # open hat "tsss"
        return shaped_noise(rng, n, u(rng, n, 4500, 7000), None, "hp") * E(u(rng, n, 0.001, 0.005), u(rng, n, 0.15, 0.5))
    if k == 5:  # "t"
        return shaped_noise(rng, n, u(rng, n, 3000, 6000), u(rng, n, 0.8, 1.5), "bp") * E(np.full(n, 0.0003), u(rng, n, 0.003, 0.008))
    if k == 6:  # "k"
        return shaped_noise(rng, n, u(rng, n, 1200, 2500), u(rng, n, 0.3, 0.7), "bp") * E(np.full(n, 0.0005), u(rng, n, 0.006, 0.015))
    if k == 7:  # "p"
        return shaped_noise(rng, n, u(rng, n, 400, 1200), None, "lp") * E(np.full(n, 0.0005), u(rng, n, 0.006, 0.02))
    if k == 8:  # "sh"
        return shaped_noise(rng, n, u(rng, n, 2000, 3500), u(rng, n, 0.4, 0.8), "bp") * E(u(rng, n, 0.01, 0.04), u(rng, n, 0.2, 0.6))
    if k == 9:  # "s"
        return shaped_noise(rng, n, u(rng, n, 4500, 6500), None, "hp") * E(u(rng, n, 0.01, 0.04), u(rng, n, 0.2, 0.6))
    if k == 10:  # "f"
        return shaped_noise(rng, n, u(rng, n, 1000, 6000), u(rng, n, 1.5, 2.5), "bp") * E(u(rng, n, 0.01, 0.04), u(rng, n, 0.2, 0.5)) * 0.6
    if k in (11, 12, 13):  # vowels "ah" "ee" "oo"
        f1, f2 = [(700, 1200), (300, 2300), (320, 800)][k - 11]
        return voice(rng, n, u(rng, n, 90, 300), vowel(rng, n, f1, f2)) * E(u(rng, n, 0.01, 0.03), u(rng, n, 0.2, 0.8))
    if k == 14:  # "m" hum
        return voice(rng, n, u(rng, n, 90, 250), [(u(rng, n, 200, 300), u(rng, n, 60, 100))]) * E(u(rng, n, 0.01, 0.03), u(rng, n, 0.2, 0.8))
    if k == 15:  # clap
        out = np.zeros((n, L))
        nz = shaped_noise(rng, n, u(rng, n, 900, 1500), u(rng, n, 0.5, 0.9), "bp")
        for j in range(3):
            out += nz * E(np.full(n, 0.0005), np.full(n, 0.005), s + j * rng.integers(120, 200, size=n))
        return out + nz * 0.6 * E(np.full(n, 0.002), u(rng, n, 0.06, 0.15), s + 500)
    if k == 16:  # finger snap
        return shaped_noise(rng, n, u(rng, n, 2000, 3500), u(rng, n, 0.2, 0.4), "bp") * E(np.full(n, 0.0002), u(rng, n, 0.004, 0.012))
    if k == 17:  # knock
        return sweep(rng, n, u(rng, n, 200, 600), u(rng, n, 200, 600), np.full(n, 1.0), s) * E(np.full(n, 0.0005), u(rng, n, 0.015, 0.05)) + 0.2 * shaped_noise(rng, n, np.full(n, 2000.0), None, "lp") * E(np.full(n, 0.0003), np.full(n, 0.003))
    if k == 18:  # whistle
        f = u(rng, n, 900, 3000)
        return sweep(rng, n, f * u(rng, n, 0.9, 1.1), f, np.full(n, 0.05), s) * E(u(rng, n, 0.01, 0.04), u(rng, n, 0.2, 0.8))
    if k == 19:  # tongue click
        return sweep(rng, n, u(rng, n, 900, 2000), u(rng, n, 900, 2000), np.full(n, 1.0), s) * E(np.full(n, 0.0003), u(rng, n, 0.004, 0.012))
    if k == 20:  # "ch"
        return shaped_noise(rng, n, u(rng, n, 2500, 4000), u(rng, n, 0.5, 0.9), "bp") * (E(np.full(n, 0.0005), np.full(n, 0.004)) + 0.6 * E(np.full(n, 0.005), u(rng, n, 0.05, 0.15)))
    if k == 21:  # "z" buzz
        return 0.6 * voice(rng, n, u(rng, n, 100, 220), [(u(rng, n, 200, 400), np.full(n, 150.0))]) * E(np.full(n, 0.01), u(rng, n, 0.2, 0.5)) + shaped_noise(rng, n, u(rng, n, 4000, 6000), None, "hp") * E(np.full(n, 0.01), u(rng, n, 0.2, 0.5)) * 0.5
    if k == 22:  # "bmm": kick into a hum
        return family(0, rng, n) + 0.5 * voice(rng, n, u(rng, n, 80, 150), [(np.full(n, 250.0), np.full(n, 80.0))]) * E(np.full(n, 0.02), np.full(n, 0.4))
    if k == 23:  # rimshot "tk!"
        return sweep(rng, n, u(rng, n, 1500, 2200), u(rng, n, 1500, 2200), np.full(n, 1.0), s) * E(np.full(n, 0.0003), u(rng, n, 0.008, 0.02)) + shaped_noise(rng, n, np.full(n, 3000.0), np.full(n, 0.8), "bp") * E(np.full(n, 0.0003), np.full(n, 0.004))
    if k == 24:  # breath "ha"
        return shaped_noise(rng, n, u(rng, n, 800, 2000), u(rng, n, 1.0, 1.8), "bp") * E(u(rng, n, 0.01, 0.03), u(rng, n, 0.1, 0.3))
    if k == 25:  # "dum": low voiced thump
        return voice(rng, n, u(rng, n, 80, 160), vowel(rng, n, 400, 900)) * E(np.full(n, 0.005), u(rng, n, 0.05, 0.15))
    if k == 26:  # "ting": high ring
        return sweep(rng, n, u(rng, n, 2000, 5000), u(rng, n, 2000, 5000), np.full(n, 1.0), s) * E(np.full(n, 0.0005), u(rng, n, 0.1, 0.4))
    if k == 27:  # lip trill "brr"
        trem = 0.5 + 0.5 * np.sin(2 * np.pi * u(rng, n, 20, 35)[:, None] * T[None, :])
        return voice(rng, n, u(rng, n, 90, 180), vowel(rng, n, 350, 800)) * trem * E(np.full(n, 0.01), u(rng, n, 0.2, 0.5))
    if k == 28:  # inward snare: slow-attack hiss
        return shaped_noise(rng, n, u(rng, n, 2000, 5000), u(rng, n, 0.6, 1.0), "bp") * E(u(rng, n, 0.02, 0.05), u(rng, n, 0.05, 0.15))
    if k == 29:  # "bah"
        return 0.6 * family(7, rng, n) + voice(rng, n, u(rng, n, 100, 250), vowel(rng, n, 700, 1200)) * E(np.full(n, 0.01), u(rng, n, 0.1, 0.3), s + 100)
    if k == 30:  # "dah"
        return 0.5 * family(5, rng, n) + voice(rng, n, u(rng, n, 100, 250), vowel(rng, n, 650, 1100)) * E(np.full(n, 0.008), u(rng, n, 0.1, 0.3), s + 80)
    if k == 31:  # "boom": long low kick
        return sweep(rng, n, u(rng, n, 90, 140), u(rng, n, 40, 55), u(rng, n, 0.05, 0.12), s) * E(u(rng, n, 0.003, 0.01), u(rng, n, 0.25, 0.6))
    raise ValueError(k)


def make(rng, per_class):
    xs, ys = [], []
    for k in range(32):
        x = family(k, rng, per_class)
        # Tone colour: a random spectral tilt (mics and mouths differ).
        s = np.fft.rfft(x, axis=1)
        f = np.fft.rfftfreq(L, 1 / F.SR)[None, :]
        tilt = rng.uniform(-6, 6, size=(per_class, 1))
        s *= 10 ** (tilt * np.log2(np.maximum(f, 50) / 1000) / 20)
        x = np.fft.irfft(s, n=L, axis=1)
        x /= np.maximum(np.abs(x).max(axis=1, keepdims=True), 1e-9)
        # A little room: one early reflection.
        d = rng.integers(40, 300, size=per_class)
        g = rng.uniform(0, 0.3, size=(per_class, 1))
        echo = np.stack([np.roll(x[i], d[i]) for i in range(per_class)])
        echo[np.arange(L)[None, :] < d[:, None]] = 0
        x = x + g * echo
        snr = rng.uniform(15, 50, size=(per_class, 1))
        x = x + rng.normal(size=x.shape) * 10 ** (-snr / 20) * x.std(axis=1, keepdims=True)
        x *= rng.uniform(0.05, 1.0, size=(per_class, 1))
        xs.append(F.mouth_features(x.astype(np.float32)))
        ys.append(np.full(per_class, k))
    return np.concatenate(xs), np.concatenate(ys)


def few_shot(embed, x, y, rng, ways=8, shots=3, trials=200):
    """Accuracy of nearest-prototype classification, as on the device."""
    e = embed(x)
    e = e / np.maximum(np.linalg.norm(e, axis=1, keepdims=True), 1e-9)
    hits = 0
    total = 0
    for _ in range(trials):
        classes = rng.choice(32, ways, replace=False)
        protos, queries = [], []
        for c in classes:
            idx = rng.permutation(np.where(y == c)[0])
            p = e[idx[:shots]].mean(axis=0)
            protos.append(p / max(np.linalg.norm(p), 1e-9))
            queries.append(idx[shots:shots + 10])
        P = np.stack(protos)
        for j, q in enumerate(queries):
            pred = (e[q] @ P.T).argmax(axis=1)
            hits += (pred == j).sum()
            total += len(q)
    return hits / total


def main():
    rng = np.random.default_rng(2)
    torch.manual_seed(2)
    per = int(sys.argv[1]) if len(sys.argv) > 1 else 3000
    x, y = make(rng, per)
    print("made", x.shape, flush=True)
    vx, vy = make(np.random.default_rng(77), 150)
    net = pmxn.Net(SPEC + [HEAD], (F.MOUTH_MELS, F.MOUTH_FRAMES))

    def embed_float(xx):
        with torch.no_grad():
            taps = []
            net(torch.tensor(xx), taps)
            return taps[-2].numpy()

    def val(net):
        with torch.no_grad():
            acc = (net(torch.tensor(vx)).argmax(1).numpy() == vy).mean()
        return f"val acc {acc * 100:.1f}%  8-way 3-shot {few_shot(embed_float, vx, vy, np.random.default_rng(3)) * 100:.1f}%"

    pmxn.train(net, x, y, epochs=int(sys.argv[2]) if len(sys.argv) > 2 else 20, lr=3e-3, val=val)
    # Ship the embedding only.
    emb = pmxn.Net(SPEC, (F.MOUTH_MELS, F.MOUTH_FRAMES))
    emb.mods.load_state_dict({k: v for k, v in net.mods.state_dict().items() if not k.startswith(str(len(emb.mods)))}, strict=True)
    q = pmxn.quantise(emb, x[np.random.default_rng(5).choice(len(x), 2000, replace=False)])
    sub = np.random.default_rng(4).choice(len(vx), 1200, replace=False)
    e8 = lambda xx: np.stack([pmxn.dequant(q, pmxn.run_int8(q, v)) for v in xx])
    print("int8 8-way 3-shot", few_shot(e8, vx[sub], vy[sub], np.random.default_rng(3), trials=100))
    macs = pmxn.export(emb, q, "mouth", "../../assets/npu", [vx[0], vx[1000], vx[3000]], meta=dict(embed=EMBED))
    import json
    clip = (np.sin(2 * np.pi * 300 * T) * np.exp(-np.maximum(T - ONSET / F.SR, 0) / 0.05) * (T >= ONSET / F.SR)).astype(np.float32)
    json.dump(dict(clip=[float(v) for v in clip], features=[float(v) for v in F.mouth_features(clip).reshape(-1)]), open("../../assets/npu/mouth.features.json", "w"))
    print("MACs per sound", macs)


if __name__ == "__main__":
    main()
