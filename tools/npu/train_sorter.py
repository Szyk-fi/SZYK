"""Sorter: a network that sorts a sample library -- what kind of sound
each sample is (kick, snare, hat, ... bass, keys, texture), and a sound
embedding for "more like this".

Input: the first ~300 ms of a sample from just before its onset, as 32 mel
bands x 24 frames (`features.sort_features`). Two networks ship:

  sorter_embed: features -> 48-number embedding (the "sounds like" space)
  sorter_head:  embedding -> 14 class scores

Training data is synthesized here, with deliberately wide variation in
each class (808-, 909- and acoustic-style kicks; noise-, tone- and
clap-heavy snares; metallic and noisy hats; ...) and augmented with
room, saturation, EQ, noise and level. Nothing comes from a dataset. The
Rust tests then check it on drums from generators it never saw (the
device's own drum synths), which is the honest measure of whether it
learned "kick" rather than "this script's kick".
"""

import json
import sys
import numpy as np
import torch

import features as F
import pmxn

L = F.SORT_SAMPLES
ONSET = F.SORT_PRE * F.SORT_HOP
SR = F.SR
EMBED = 48
CLASSES = ["kick", "snare", "clap", "closed hat", "open hat", "cymbal", "tom", "rim", "metal", "shaker", "hand perc", "bass", "tonal", "texture"]
C = len(CLASSES)

SPEC = [
    ("conv", F.SORT_MELS, 48, 3, 1, 1, True),
    ("pool", 2),
    ("conv", 48, 64, 3, 1, 1, True),
    ("pool", 2),
    ("conv", 64, 64, 3, 2, 1, True),
    ("flatten",),
    ("dense", 64 * 3, EMBED, True),
]
HEAD = [("dense", EMBED, C, False)]

T = np.arange(L) / SR


def u(rng, n, a, b):
    return rng.uniform(a, b, size=n)


def lu(rng, n, a, b):
    """Log-uniform."""
    return np.exp(rng.uniform(np.log(a), np.log(b), size=n))


def env(n, attack, decay, start, hold=None):
    tt = T[None, :] - start[:, None] / SR
    a = np.maximum(attack[:, None], 1e-4)
    d = np.maximum(decay[:, None], 1e-4)
    h = 0 if hold is None else hold[:, None]
    e = np.where(tt < 0, 0.0, np.where(tt < a, tt / a, np.where(tt < a + h, 1.0, np.exp(-(tt - a - h) / d))))
    return e


def noise(rng, n, lo=None, hi=None):
    """White noise, optionally band-limited between lo and hi Hz (soft
    edges), normalised."""
    w = rng.normal(size=(n, L))
    if lo is None and hi is None:
        return w
    s = np.fft.rfft(w, axis=1)
    f = np.fft.rfftfreq(L, 1 / SR)[None, :]
    g = np.ones_like(f)
    if lo is not None:
        g = g / (1 + (lo[:, None] / np.maximum(f, 1)) ** 4)
    if hi is not None:
        g = g / (1 + (f / hi[:, None]) ** 4)
    out = np.fft.irfft(s * g, n=L, axis=1)
    return out / np.maximum(out.std(axis=1, keepdims=True), 1e-9)


def sweep(n, f0, f1, tau, start, shape="sin"):
    tt = np.maximum(T[None, :] - start[:, None] / SR, 0)
    f = f1[:, None] + (f0 - f1)[:, None] * np.exp(-tt / np.maximum(tau[:, None], 1e-4))
    ph = np.cumsum(f, axis=1) / SR
    if shape == "tri":
        return 2 * np.abs(2 * (ph % 1) - 1) - 1
    return np.sin(2 * np.pi * ph)


def osc(rng, n, f, kind, start):
    """Harmonic oscillators: 0 sine, 1 saw, 2 square, 3 triangle-ish
    (additive to 7 kHz, so no aliasing)."""
    out = np.zeros((n, L))
    ph = np.maximum(T[None, :] - start[:, None] / SR, 0) * f[:, None]
    for h in range(1, 80):
        fh = f * h
        amp = np.where(kind == 0, (h == 1) * 1.0, np.where(kind == 1, 1.0 / h, np.where(kind == 2, (h % 2) / h, (h % 2) / h ** 2)))
        amp = np.where(fh < 7000, amp, 0)
        if not amp.any():
            break
        out += amp[:, None] * np.sin(2 * np.pi * h * ph)
    return out


def metallic(rng, n, base, start):
    """The 808 hat/cymbal recipe: six square waves at inharmonic ratios."""
    ratios = np.array([2.0, 3.0, 4.16, 5.43, 6.79, 8.21])
    out = np.zeros((n, L))
    tt = np.maximum(T[None, :] - start[:, None] / SR, 0)
    for r in ratios:
        jit = u(rng, n, 0.97, 1.03)
        out += np.sign(np.sin(2 * np.pi * (base * r * jit)[:, None] * tt))
    return out / 6


def bp(x, lo, hi):
    s = np.fft.rfft(x, axis=1)
    f = np.fft.rfftfreq(L, 1 / SR)[None, :]
    g = 1 / (1 + (lo[:, None] / np.maximum(f, 1)) ** 4) / (1 + (f / hi[:, None]) ** 4)
    return np.fft.irfft(s * g, n=L, axis=1)


def pick(rng, n, *opts):
    """Per example, one of several generators."""
    k = rng.integers(0, len(opts), size=n)
    out = np.zeros((n, L))
    for i, o in enumerate(opts):
        m = k == i
        if m.any():
            out[m] = o(m.sum())
    return out


def klass(c, rng, n):
    s = ONSET + rng.integers(-30, 60, size=n)
    E = lambda a, d, n_=n, st=None, hold=None: env(n_, a, d, s if st is None else st, hold)
    if c == 0:  # kick
        def k808(m):
            st = s[:m]
            return sweep(m, lu(rng, m, 70, 200), lu(rng, m, 38, 62), lu(rng, m, 0.01, 0.06), st) * env(m, u(rng, m, 0.0005, 0.003), lu(rng, m, 0.15, 0.9), st)
        def k909(m):
            st = s[:m]
            body = sweep(m, lu(rng, m, 150, 400), lu(rng, m, 45, 75), lu(rng, m, 0.005, 0.03), st, "tri") * env(m, u(rng, m, 0.0005, 0.002), lu(rng, m, 0.08, 0.35), st)
            click = noise(rng, m, None, lu(rng, m, 2000, 6000)) * env(m, np.full(m, 0.0002), lu(rng, m, 0.002, 0.008), st)
            return body + u(rng, m, 0.1, 0.6)[:, None] * click
        def acoustic(m):
            st = s[:m]
            thump = noise(rng, m, None, lu(rng, m, 120, 300)) * env(m, np.full(m, 0.001), lu(rng, m, 0.04, 0.12), st)
            tone = sweep(m, lu(rng, m, 90, 140), lu(rng, m, 50, 70), np.full(m, 0.02), st) * env(m, np.full(m, 0.001), lu(rng, m, 0.06, 0.2), st)
            beater = noise(rng, m, lu(rng, m, 1500, 3000), lu(rng, m, 4000, 8000)) * env(m, np.full(m, 0.0002), lu(rng, m, 0.002, 0.006), st)
            return thump * 0.6 + tone + u(rng, m, 0.1, 0.5)[:, None] * beater
        return pick(rng, n, k808, k909, acoustic)
    if c == 1:  # snare
        f = lu(rng, n, 150, 280)
        tone = sweep(n, f * u(rng, n, 1.1, 1.6), f, lu(rng, n, 0.005, 0.03), s) + 0.5 * np.sin(2 * np.pi * (f * u(rng, n, 1.5, 1.9))[:, None] * np.maximum(T[None, :] - s[:, None] / SR, 0))
        tone *= E(np.full(n, 0.0005), lu(rng, n, 0.03, 0.12))
        nz = noise(rng, n, lu(rng, n, 800, 3000), lu(rng, n, 6000, 9000)) * E(np.full(n, 0.0005), lu(rng, n, 0.06, 0.35))
        mix = u(rng, n, 0.2, 0.85)[:, None]
        return tone * (1 - mix) + nz * mix * 1.3
    if c == 2:  # clap
        out = np.zeros((n, L))
        nz = noise(rng, n, lu(rng, n, 700, 1500), lu(rng, n, 2000, 5000))
        bursts = rng.integers(2, 5, size=n)
        gap = lu(rng, n, 0.006, 0.015) * SR
        for j in range(4):
            on = (j < bursts)[:, None]
            out += on * nz * E(np.full(n, 0.0003), lu(rng, n, 0.003, 0.008), st=s + (j * gap).astype(int))
        tail = nz * u(rng, n, 0.3, 0.8)[:, None] * E(np.full(n, 0.002), lu(rng, n, 0.08, 0.4), st=s + (bursts * gap).astype(int))
        return out + tail
    if c in (3, 4):  # closed / open hat
        dec = lu(rng, n, 0.01, 0.06) if c == 3 else lu(rng, n, 0.18, 0.9)
        met = metallic(rng, n, lu(rng, n, 250, 600), s)
        nz = noise(rng, n)
        mix = u(rng, n, 0, 1)[:, None]
        x = bp(met * mix + nz * (1 - mix), lu(rng, n, 5000, 8000), np.full(n, 20000.0))
        x /= np.maximum(x.std(axis=1, keepdims=True), 1e-9)
        return x * E(u(rng, n, 0.0003, 0.002), dec)
    if c == 5:  # cymbal: crash, ride
        def crash(m):
            st = s[:m]
            x = metallic(rng, m, lu(rng, m, 200, 500), st) * 0.6 + noise(rng, m, lu(rng, m, 2000, 4000), None)
            x = bp(x, lu(rng, m, 1500, 4000), np.full(m, 20000.0))
            x /= np.maximum(x.std(axis=1, keepdims=True), 1e-9)
            return x * env(m, u(rng, m, 0.001, 0.01), lu(rng, m, 0.8, 3.0), st)
        def ride(m):
            st = s[:m]
            ping = np.zeros((m, L))
            tt = np.maximum(T[None, :] - st[:, None] / SR, 0)
            for r in [1.0, 2.37, 3.81, 5.12]:
                ping += np.sin(2 * np.pi * (lu(rng, m, 2500, 5000) * r / 2.37)[:, None] * tt) / r
            wash = noise(rng, m, lu(rng, m, 4000, 7000), None)
            return (ping * 0.8 + wash * 0.5) * env(m, np.full(m, 0.0005), lu(rng, m, 0.6, 2.0), st)
        return pick(rng, n, crash, ride)
    if c == 6:  # tom
        f = lu(rng, n, 70, 320)
        body = sweep(n, f * u(rng, n, 1.2, 2.0), f, lu(rng, n, 0.02, 0.1), s) * E(np.full(n, 0.0007), lu(rng, n, 0.12, 0.6))
        stick = noise(rng, n, lu(rng, n, 500, 2000), lu(rng, n, 3000, 6000)) * E(np.full(n, 0.0003), lu(rng, n, 0.004, 0.02))
        return body + u(rng, n, 0.05, 0.4)[:, None] * stick
    if c == 7:  # rim / stick / click
        f = lu(rng, n, 400, 2200)
        tt = np.maximum(T[None, :] - s[:, None] / SR, 0)
        tone = np.sin(2 * np.pi * f[:, None] * tt) + 0.5 * np.sin(2 * np.pi * (f * u(rng, n, 1.4, 2.6))[:, None] * tt)
        x = tone * E(np.full(n, 0.0002), lu(rng, n, 0.006, 0.03)) + noise(rng, n, lu(rng, n, 1500, 3000), None) * E(np.full(n, 0.0002), lu(rng, n, 0.002, 0.008)) * u(rng, n, 0.2, 1.0)[:, None]
        return x
    if c == 8:  # metal: cowbell, agogo, bell, triangle
        def cowbell(m):
            st = s[:m]
            f = lu(rng, m, 450, 900)
            tt = np.maximum(T[None, :] - st[:, None] / SR, 0)
            x = np.sign(np.sin(2 * np.pi * f[:, None] * tt)) + np.sign(np.sin(2 * np.pi * (f * u(rng, m, 1.45, 1.52))[:, None] * tt))
            x = bp(x, f * 0.8, f * 4)
            return x * env(m, np.full(m, 0.0005), lu(rng, m, 0.06, 0.4), st)
        def bell(m):
            st = s[:m]
            f = lu(rng, m, 600, 3500)
            tt = np.maximum(T[None, :] - st[:, None] / SR, 0)
            x = np.zeros((m, L))
            for r, a in [(1.0, 1.0), (2.76, 0.6), (5.4, 0.4), (8.93, 0.25)]:
                x += a * np.sin(2 * np.pi * (f * r * u(rng, m, 0.98, 1.02))[:, None] * tt)
            return x * env(m, np.full(m, 0.0003), lu(rng, m, 0.15, 1.5), st)
        return pick(rng, n, cowbell, bell)
    if c == 9:  # shaker / tambourine
        def shaker(m):
            st = s[:m]
            x = noise(rng, m, lu(rng, m, 3000, 7000), None)
            return x * env(m, lu(rng, m, 0.008, 0.04), lu(rng, m, 0.02, 0.12), st)
        def tamb(m):
            st = s[:m]
            jing = metallic(rng, m, lu(rng, m, 1000, 2000), st) * 0.5 + noise(rng, m, np.full(m, 4000.0), None)
            x = bp(jing, np.full(m, 3500.0), np.full(m, 20000.0))
            x /= np.maximum(x.std(axis=1, keepdims=True), 1e-9)
            return x * (env(m, np.full(m, 0.001), lu(rng, m, 0.04, 0.2), st) + 0.5 * env(m, np.full(m, 0.001), lu(rng, m, 0.02, 0.06), st + rng.integers(400, 1600, size=m)))
        return pick(rng, n, shaker, tamb)
    if c == 10:  # hand perc: conga, bongo, woodblock, clave
        def drum(m):
            st = s[:m]
            f = lu(rng, m, 150, 550)
            body = sweep(m, f * u(rng, m, 1.05, 1.3), f, np.full(m, 0.01), st) * env(m, np.full(m, 0.0005), lu(rng, m, 0.06, 0.3), st)
            slap = noise(rng, m, lu(rng, m, 600, 1500), lu(rng, m, 3000, 6000)) * env(m, np.full(m, 0.0003), lu(rng, m, 0.003, 0.012), st)
            return body + u(rng, m, 0.1, 0.8)[:, None] * slap
        def wood(m):
            st = s[:m]
            f = lu(rng, m, 600, 2000)
            tt = np.maximum(T[None, :] - st[:, None] / SR, 0)
            return (np.sin(2 * np.pi * f[:, None] * tt) + 0.3 * np.sin(2 * np.pi * (f * 2.7)[:, None] * tt)) * env(m, np.full(m, 0.0002), lu(rng, m, 0.02, 0.07), st)
        return pick(rng, n, drum, wood)
    if c == 11:  # bass: a note below ~160 Hz
        f = lu(rng, n, 35, 160)
        kind = rng.integers(0, 4, size=n)
        x = osc(rng, n, f, kind, s)
        x = bp(x, np.full(n, 20.0), lu(rng, n, 200, 3000))
        return x * E(lu(rng, n, 0.001, 0.02), lu(rng, n, 0.25, 2.0), hold=u(rng, n, 0, 0.3))
    if c == 12:  # tonal: keys, plucks, leads, stabs, mallets (harmonic)
        f = lu(rng, n, 160, 1500)
        kind = rng.integers(0, 4, size=n)
        x = osc(rng, n, f, kind, s)
        # some chords: a third and a fifth on top
        chord = (rng.uniform(size=n) < 0.35)[:, None]
        x = x + chord * (osc(rng, n, f * 2 ** (rng.choice([3, 4], size=n) / 12), kind, s) + osc(rng, n, f * 2 ** (7 / 12), kind, s))
        x = bp(x, np.full(n, 60.0), lu(rng, n, 800, 8000))
        return x * E(lu(rng, n, 0.001, 0.03), lu(rng, n, 0.1, 1.5), hold=u(rng, n, 0, 0.2))
    if c == 13:  # texture: pads, swells, noise sweeps, drones
        def pad(m):
            st = s[:m]
            f = lu(rng, m, 80, 800)
            x = osc(rng, m, f, rng.integers(1, 4, size=m), st) + osc(rng, m, f * u(rng, m, 1.003, 1.01), rng.integers(1, 4, size=m), st)
            return bp(x, np.full(m, 60.0), lu(rng, m, 400, 3000)) * env(m, lu(rng, m, 0.12, 0.8), np.full(m, 2.0), st)
        def swell(m):
            st = s[:m]
            x = noise(rng, m, lu(rng, m, 200, 2000), lu(rng, m, 2500, 9000))
            return x * env(m, lu(rng, m, 0.15, 1.0), np.full(m, 1.0), st)
        def drone(m):
            st = s[:m]
            x = noise(rng, m, lu(rng, m, 60, 300), lu(rng, m, 400, 1200)) * 0.5 + osc(rng, m, lu(rng, m, 40, 120), np.zeros(m, int), st)
            return x * env(m, lu(rng, m, 0.05, 0.4), np.full(m, 3.0), st)
        return pick(rng, n, pad, swell, drone)
    raise ValueError(c)


def augment(rng, x):
    n = len(x)
    # EQ: a random tilt and an occasional low or high cut (mics, sample
    # packs and old samplers all colour things)
    s = np.fft.rfft(x, axis=1)
    f = np.fft.rfftfreq(L, 1 / SR)[None, :]
    tilt = rng.uniform(-5, 5, size=(n, 1))
    s *= 10 ** (tilt * np.log2(np.maximum(f, 40) / 1000) / 20)
    lp = np.where(rng.uniform(size=n) < 0.25, lu(rng, n, 2500, 8000), 1e9)
    hp = np.where(rng.uniform(size=n) < 0.15, lu(rng, n, 40, 400), 1.0)
    s /= (1 + (f / lp[:, None]) ** 4)
    s /= (1 + (hp[:, None] / np.maximum(f, 1)) ** 4)
    x = np.fft.irfft(s, n=L, axis=1)
    x /= np.maximum(np.abs(x).max(axis=1, keepdims=True), 1e-9)
    # saturation
    drive = np.where(rng.uniform(size=(n, 1)) < 0.3, lu(rng, n, 1.5, 6.0)[:, None], 1.0)
    x = np.tanh(x * drive) / np.tanh(drive)
    # room: a decaying noise tail convolved in the frequency domain
    wet = np.where(rng.uniform(size=n) < 0.4, u(rng, n, 0.05, 0.5), 0.0)
    if wet.any():
        rt = lu(rng, n, 0.05, 0.6)
        ir = rng.normal(size=(n, L)) * np.exp(-T[None, :] / rt[:, None])
        ir[:, :rng.integers(20, 200)] = 0
        ir /= np.maximum(np.sqrt((ir ** 2).sum(axis=1, keepdims=True)), 1e-9)
        rev = np.fft.irfft(np.fft.rfft(x, n=2 * L, axis=1) * np.fft.rfft(ir, n=2 * L, axis=1), n=2 * L, axis=1)[:, :L]
        x = x + wet[:, None] * rev
    # noise floor and level
    snr = rng.uniform(25, 70, size=(n, 1))
    x = x + rng.normal(size=x.shape) * 10 ** (-snr / 20) * np.maximum(x.std(axis=1, keepdims=True), 1e-6)
    x *= lu(rng, n, 0.05, 1.0)[:, None]
    return x


def make(rng, per_class):
    xs, ys = [], []
    for c in range(C):
        x = augment(rng, klass(c, rng, per_class))
        xs.append(F.sort_features(x.astype(np.float32)))
        ys.append(np.full(per_class, c))
        print(" ", CLASSES[c], flush=True)
    return np.concatenate(xs), np.concatenate(ys)


def main():
    rng = np.random.default_rng(11)
    torch.manual_seed(11)
    per = int(sys.argv[1]) if len(sys.argv) > 1 else 2000
    epochs = int(sys.argv[2]) if len(sys.argv) > 2 else 16
    x, y = make(rng, per)
    print("made", x.shape, flush=True)
    vx, vy = make(np.random.default_rng(99), 200)
    net = pmxn.Net(SPEC + HEAD, (F.SORT_MELS, F.SORT_FRAMES))

    def val(net):
        with torch.no_grad():
            p = net(torch.tensor(vx)).argmax(1).numpy()
        per_class = [f"{CLASSES[c][:5]} {(p[vy == c] == c).mean() * 100:.0f}" for c in range(C)]
        return f"val {(p == vy).mean() * 100:.1f}%  " + " ".join(per_class)

    pmxn.train(net, x, y, epochs=epochs, lr=3e-3, val=val)

    # Split into the two shipped networks.
    sd = net.mods.state_dict()
    emb = pmxn.Net(SPEC, (F.SORT_MELS, F.SORT_FRAMES))
    n_emb = len(emb.mods)
    emb.mods.load_state_dict({k: v for k, v in sd.items() if int(k.split(".")[0]) < n_emb}, strict=True)
    head = pmxn.Net(HEAD, (EMBED, 1))
    head.mods.load_state_dict({f"0.{k.split('.', 1)[1]}": v for k, v in sd.items() if int(k.split(".")[0]) == n_emb}, strict=True)

    calib = x[np.random.default_rng(5).choice(len(x), min(3000, len(x)), replace=False)]
    qe = pmxn.quantise(emb, calib)
    with torch.no_grad():
        ecal = emb(torch.tensor(calib)).numpy().reshape(-1, EMBED, 1)
    qh = pmxn.quantise(head, ecal)

    sub = np.random.default_rng(4).choice(len(vx), min(1400, len(vx)), replace=False)
    e8 = np.stack([pmxn.dequant(qe, pmxn.run_int8(qe, v)) for v in vx[sub]])
    p8 = np.stack([pmxn.dequant(qh, pmxn.run_int8(qh, e.reshape(EMBED, 1))) for e in e8]).argmax(1)
    print(f"int8 accuracy {(p8 == vy[sub]).mean() * 100:.1f}%")

    me = pmxn.export(emb, qe, "sorter_embed", "../../assets/npu", [vx[0], vx[len(vx) // 3], vx[2 * len(vx) // 3]], meta=dict(embed=EMBED))
    with torch.no_grad():
        etests = emb(torch.tensor(vx[[0, len(vx) // 3, 2 * len(vx) // 3]])).numpy().reshape(-1, EMBED, 1)
    mh = pmxn.export(head, qh, "sorter_head", "../../assets/npu", list(etests), meta=dict(classes=CLASSES))
    clip = (np.sin(2 * np.pi * 180 * T) * np.exp(-np.maximum(T - ONSET / SR, 0) / 0.1) * (T >= ONSET / SR)).astype(np.float32)
    json.dump(dict(clip=[float(v) for v in clip], features=[float(v) for v in F.sort_features(clip).reshape(-1)]), open("../../assets/npu/sorter.features.json", "w"))
    print("MACs per sample", me + mh)


if __name__ == "__main__":
    main()
