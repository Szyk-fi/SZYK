"""Timbre Map: a neural synthesizer with a map of sounds.

A variational autoencoder learns a 2-D map of instrument timbres. Only its
decoder ships: given a point on the map, the note, the velocity and the
time since the note started, it outputs one control frame of sound --
32 harmonic levels, 4 noise-band levels and a loudness -- which the app
turns into audio with an additive synthesizer (the DDSP approach). It runs
every 4 ms for every sounding voice.

Training data: 16 families of instrument models written here from their
acoustics -- brass (brightness rising with loudness), bowed strings (body
resonances, bow noise), flute (strong fundamental, breath), clarinet (odd
harmonics), double reeds (a formant near 1.2 kHz), organ drawbars, sung
vowels, plucked strings (pluck-position comb, high harmonics dying first),
piano-like strikes, mallets, saw and square synths with filter sweeps,
sine pads, kazoo, bassoon and harmonica. Each instance draws its own
parameters; the encoder sees four frames of an instance at middle C and
places it on the map, so similar instruments land near each other.
"""

import json
import sys
import numpy as np
import torch
import torch.nn as nn

import pmxn

H = 32
NB = 4
OUT = H + NB + 1
SIG_T = [0.01, 0.05, 0.2, 1.0]
FAMILIES = ["Brass", "Strings", "Flute", "Clarinet", "Oboe", "Organ", "Choir", "Plucked", "Piano", "Mallet", "Saw synth", "Square synth", "Sine pad", "Kazoo", "Bassoon", "Harmonica"]
DEC_SPEC = [("dense", 5, 64, True), ("dense", 64, 128, True), ("dense", 128, 128, True), ("dense", 128, OUT, False)]
NOISE_BANDS = [(0, 1000), (1000, 3000), (3000, 6000), (6000, 12000)]


def time_feature(t):
    return np.log2(1 + np.asarray(t) / 0.01) / 8.0


def formant(freqs, peaks):
    e = np.full(freqs.shape, 0.05)
    for fc, bw, g in peaks:
        e = e + g / (1 + ((freqs - fc) / bw) ** 2)
    return e


class Instance:
    """One instrument: a family plus its own random parameters."""

    def __init__(self, fam, rng):
        self.fam = fam
        r = lambda a, b: rng.uniform(a, b)
        self.p = dict(
            tilt=r(0.6, 1.6), attack=r(0.01, 0.12), decay=r(0.3, 3.0), body=[(r(200, 600), r(80, 200), r(0.5, 1.5)), (r(800, 1500), r(150, 300), r(0.3, 1)), (r(2000, 3500), r(300, 600), r(0.2, 0.8))],
            breath=r(0.02, 0.2), draw=rng.uniform(0, 1, 6) * (rng.random(6) < 0.6), vowel=rng.integers(0, 5), beta=r(0.08, 0.3), cutoff=r(2, 12), sweep=r(0.05, 0.5), formant=r(900, 1600), nasal=r(1300, 2500), vib=r(0, 0.6),
        )

    def frame(self, midi, vel, t):
        """(harmonic dB [H], noise dB [NB], loudness dB) for one moment."""
        f0 = 440 * 2 ** ((midi - 69) / 12)
        h = np.arange(1, H + 1)
        fr = f0 * h
        p, fam = self.p, self.fam
        att = min(t / p["attack"], 1.0)
        noise = np.full(NB, -60.0)
        if fam == 0:  # brass: brighter when louder and as the note opens
            tilt = 2.6 - 1.6 * vel * (0.4 + 0.6 * att)
            amp = h ** -tilt * formant(fr, [(1200, 600, 1.0)])
            loud = att * (0.6 + 0.4 * vel)
        elif fam == 1:  # strings
            amp = h ** -1.0 * formant(fr, p["body"])
            noise[2] = -30 + 10 * vel
            loud = min(t / max(p["attack"] * 2, 0.06), 1)
        elif fam == 2:  # flute
            amp = np.exp(-(h - 1) * 1.1)
            noise[1] = noise[2] = -22 - 20 * (1 - p["breath"] * 4)
            loud = min(t / max(p["attack"], 0.04), 1) * (1 + 0.05 * np.sin(2 * np.pi * 5 * t) * p["vib"])
        elif fam == 3:  # clarinet
            amp = np.where(h % 2 == 1, h ** -0.9, h ** -2.5 * 0.1)
            amp = np.where(h > 10, h ** -1.5 * 0.3, amp)
            loud = att
        elif fam == 4:  # oboe / double reed
            amp = h ** -0.6 * formant(fr, [(p["formant"], 400, 2.0)])
            loud = att
        elif fam == 5:  # organ drawbars
            amp = np.full(H, 1e-4)
            for i, hh in enumerate([1, 2, 3, 4, 6, 8]):
                amp[hh - 1] += p["draw"][i] + (0.6 if hh == 1 else 0)
            loud = min(t / 0.005, 1)
        elif fam == 6:  # sung vowels
            F = [[(700, 110, 1), (1200, 120, 0.7), (2600, 160, 0.3)], [(400, 80, 1), (2000, 120, 0.6), (2600, 160, 0.4)], [(300, 70, 1), (2300, 120, 0.7), (3000, 160, 0.4)], [(450, 80, 1), (800, 100, 0.7), (2600, 160, 0.2)], [(320, 70, 1), (750, 90, 0.6), (2400, 160, 0.1)]][p["vowel"]]
            amp = h ** -0.8 * formant(fr, F)
            noise[2] = -40
            loud = min(t / max(p["attack"] * 3, 0.08), 1)
        elif fam == 7:  # plucked string
            amp = np.abs(np.sin(np.pi * h * p["beta"])) / h ** 1.2 * np.exp(-t * (0.5 + 0.02 * h ** 2) / p["decay"] * 2)
            noise[1] = noise[2] = -20 if t < 0.02 else -60
            loud = np.exp(-t / p["decay"])
        elif fam == 8:  # piano-like
            amp = np.abs(np.sin(np.pi * h / 7.0)) / h ** 1.1 * np.exp(-t * 0.05 * h)
            noise[0] = noise[1] = -15 if t < 0.015 else -60
            loud = 0.6 * np.exp(-t / 0.4) + 0.4 * np.exp(-t / (p["decay"] * 2))
        elif fam == 9:  # soft mallet
            amp = np.exp(-(h - 1) * 2.0)
            noise[1] = -25 if t < 0.01 else -60
            loud = np.exp(-t / (p["decay"] * 0.3))
        elif fam == 10:  # saw synth with a filter sweep
            cutoff = f0 * (1 + p["cutoff"] * (vel * np.exp(-t / p["sweep"]) + 0.2))
            amp = (1 / h) / np.sqrt(1 + (fr / cutoff) ** 4)
            loud = min(t / 0.01, 1)
        elif fam == 11:  # square synth
            cutoff = f0 * (2 + p["cutoff"] * np.exp(-t / p["sweep"]))
            amp = np.where(h % 2 == 1, 1 / h, 1e-4) / np.sqrt(1 + (fr / cutoff) ** 4)
            loud = min(t / 0.005, 1)
        elif fam == 12:  # sine pad
            amp = np.exp(-(h - 1) * 3.0)
            loud = min(t / max(p["attack"] * 6, 0.2), 1)
        elif fam == 13:  # kazoo
            amp = h ** -0.3 * formant(fr, [(p["nasal"], 300, 3.0)])
            noise[2] = -30
            loud = att
        elif fam == 14:  # bassoon
            amp = h ** -0.9 * formant(fr, [(450, 150, 2.0), (1100, 300, 0.6)])
            loud = att
        else:  # harmonica
            amp = h ** -0.5 * formant(fr, [(1600, 500, 1.0)])
            noise[1] = -35
            loud = min(t / 0.03, 1)
        amp = np.where(fr < 16000, amp, 1e-6)
        db = 20 * np.log10(np.maximum(amp, 1e-6))
        db = np.maximum(db - db.max(), -60)
        loud_db = max(20 * np.log10(max(loud, 1e-6)), -60)
        return db, np.clip(noise - 0.0, -60, 30), loud_db


def vec(db, noise, loud):
    return np.concatenate([db / 60.0, noise / 60.0, [loud / 60.0]]).astype(np.float32)


def batch(rng, n_inst, k):
    sig, cond, target, fams = [], [], [], []
    for _ in range(n_inst):
        fam = rng.integers(0, len(FAMILIES))
        inst = Instance(fam, rng)
        sig.append(np.concatenate([vec(*inst.frame(60, 0.8, tt)) for tt in SIG_T]))
        c, y = [], []
        for _ in range(k):
            midi = rng.uniform(36, 96)
            vel = rng.uniform(0.2, 1.0)
            tt = np.exp(rng.uniform(np.log(0.002), np.log(4.0)))
            c.append([(midi - 66) / 30.0, vel, time_feature(tt)])
            y.append(vec(*inst.frame(midi, vel, tt)))
        cond.append(c)
        target.append(y)
        fams.append(fam)
    return np.array(sig, np.float32), np.array(cond, np.float32), np.array(target, np.float32), np.array(fams)


def main():
    rng = np.random.default_rng(4)
    torch.manual_seed(4)
    n_inst = int(sys.argv[1]) if len(sys.argv) > 1 else 20000
    K = 24
    sig, cond, tgt, fams = batch(rng, n_inst, K)
    print("made", sig.shape, flush=True)
    enc = nn.Sequential(nn.Linear(len(SIG_T) * OUT, 128), nn.ReLU(), nn.Linear(128, 64), nn.ReLU(), nn.Linear(64, 4))
    dec = pmxn.Net(DEC_SPEC, (5, 1))
    params = list(enc.parameters()) + list(dec.parameters())
    epochs = int(sys.argv[2]) if len(sys.argv) > 2 else 40
    opt = torch.optim.AdamW(params, lr=2e-3, weight_decay=1e-5)
    S, C, Y = map(torch.tensor, (sig, cond, tgt))
    bs = 128
    steps = epochs * ((n_inst + bs - 1) // bs)
    sched = torch.optim.lr_scheduler.OneCycleLR(opt, max_lr=2e-3, total_steps=steps)
    # Loudness and the strong harmonics matter most to the ear.
    w = torch.ones(OUT)
    w[:8] = 3.0
    w[-1] = 3.0
    for ep in range(epochs):
        perm = torch.randperm(n_inst)
        tot = 0.0
        beta = 0.02 * min(1.0, ep / 10)
        for i in range(0, n_inst, bs):
            idx = perm[i:i + bs]
            mu_lv = enc(S[idx])
            mu, lv = mu_lv[:, :2], mu_lv[:, 2:].clamp(-8, 4)
            z = mu + torch.randn_like(mu) * torch.exp(0.5 * lv)
            zin = z[:, None, :].expand(-1, K, -1)
            x = torch.cat([zin, C[idx]], dim=2).reshape(-1, 5, 1)
            out = dec(x).reshape(len(idx), K, OUT)
            rec = (((out - Y[idx]) ** 2) * w).mean()
            kl = (-0.5 * (1 + lv - mu ** 2 - lv.exp())).sum(1).mean()
            loss = rec + beta * kl / OUT
            opt.zero_grad()
            loss.backward()
            opt.step()
            sched.step()
            tot += float(rec.detach()) * len(idx)
        print(f"epoch {ep + 1}/{epochs} recon {tot / n_inst:.5f} (dB err ~{np.sqrt(tot / n_inst) * 60:.1f})", flush=True)
    enc.eval()
    dec.eval()
    with torch.no_grad():
        mu = enc(S)[:, :2].numpy()
    marks = []
    for f, name in enumerate(FAMILIES):
        c = np.median(mu[fams == f], axis=0)
        marks.append(dict(name=name, x=float(c[0]), y=float(c[1])))
        print(name, c)
    lo, hi = np.quantile(mu, 0.01, axis=0), np.quantile(mu, 0.99, axis=0)
    zs = rng.uniform(lo, hi, size=(3000, 2))
    cs = cond.reshape(-1, 3)[rng.choice(n_inst * K, 3000)]
    calib = np.concatenate([zs, cs], axis=1).astype(np.float32).reshape(-1, 5, 1)
    q = pmxn.quantise(dec, calib)
    with torch.no_grad():
        f_out = dec(torch.tensor(calib[:500])).numpy()
    i_out = np.stack([pmxn.dequant(q, pmxn.run_int8(q, v)) for v in calib[:500]])
    print("int8 vs float, dB rms", np.sqrt(((f_out - i_out) ** 2).mean()) * 60)
    macs = pmxn.export(dec, q, "timbre", "../../assets/npu", [calib[0], calib[1], calib[2]], meta=dict(harmonics=H, noise_bands=[list(b) for b in NOISE_BANDS], families=marks, map_lo=[float(v) for v in lo], map_hi=[float(v) for v in hi], pitch_center=66, pitch_span=30, time_unit=0.01, time_div=8.0))
    print("MACs per frame", macs)


if __name__ == "__main__":
    main()
