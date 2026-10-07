#!/usr/bin/env python3
"""Compares a Blaster character's rendered WAV with a recording of the sound it
imitates: the pitch rise, the held tone and the release, as numbers and a
picture (/tmp/compare.png). Needs numpy, scipy and matplotlib.

    BLASTER_WRITE_WAVS=/tmp/b cargo test --bin portamax-sim render_demo_wavs
    python3 tools/compare_blaster_to_recording.py recording.mp3 /tmp/b/01_X_Charge_Shot.wav \
        --ref-press 0.70 --ref-release 9.94 --held 3.0 9.0

The recording is decoded with ffmpeg. --ref-press is when the charge starts in
the recording, --ref-release when the tone stops, --held a stretch of the held
tone. Blaster's render holds the key for 1.15 x the charge time, after 0.15 s.
"""
import argparse, subprocess, wave, tempfile, os
import numpy as np
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from scipy.signal import stft

ap = argparse.ArgumentParser()
ap.add_argument("recording"); ap.add_argument("render")
ap.add_argument("--ref-press", type=float, default=0.70)
ap.add_argument("--ref-release", type=float, default=9.94)
ap.add_argument("--held", type=float, nargs=2, default=[3.0, 9.0])
ap.add_argument("--charge-time", type=float, default=1.4)
ap.add_argument("--out", default="/tmp/compare.png")
a = ap.parse_args()
fs = 48000

def load(path):
    if not path.endswith(".wav"):
        tmp = tempfile.mktemp(suffix=".wav")
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", path, "-ac", "1", "-ar", str(fs), tmp], check=True)
        path = tmp
    w = wave.open(path)
    x = np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16).astype(float) / 32768
    return x.reshape(-1, w.getnchannels()).mean(1)

ref, me = load(a.recording), load(a.render)
press_me, release_me = 0.15, 0.15 + a.charge_time * 1.15
cands = np.arange(150, 620, 1.0)

def hps(seg):
    n = 4096
    seg = np.pad(seg, (0, n - len(seg)))
    sp = np.log(np.abs(np.fft.rfft(seg * np.hanning(n))) + 1e-9)
    sc = [sum(sp[int(round(c * k * n / fs)) - 1:int(round(c * k * n / fs)) + 2].max() for k in range(1, 9)) for c in cands]
    return cands[int(np.argmax(sc))]

print("pitch rise: seconds since press -> fundamental Hz (recording | render)")
for dt in np.arange(0.0, a.charge_time + 0.2, 0.1):
    r = hps(ref[int((a.ref_press + dt) * fs):][:int(0.06 * fs)])
    m = hps(me[int((press_me + dt) * fs):][:int(0.06 * fs)])
    print(f"  {dt:.1f}s  {r:5.0f} | {m:5.0f}")

def release(x, t0, span=0.8):
    seg = x[int(t0 * fs):int((t0 + span) * fs)]
    w = int(0.01 * fs)
    e = np.sqrt(np.convolve(seg ** 2, np.ones(w) / w, "same"))
    f, t, Z = stft(seg, fs=fs, nperseg=1024, noverlap=768)
    S = np.abs(Z) ** 2
    cen = (S * f[:, None]).sum(0) / (S.sum(0) + 1e-12)
    return (round(float(e.max()), 4), round(float(np.argmax(e) / fs), 2),
            [round(float(e[int(tt * fs)]), 4) for tt in (0.05, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6)],
            [int(cen[np.argmin(abs(t - tt))]) for tt in (0.08, 0.15, 0.25, 0.35, 0.45)])

print("\nrelease: peak, peak time, rms at .05/.1/.2/.3/.4/.5/.6 s, spectral centroid at .08/.15/.25/.35/.45 s")
print("  recording", *release(ref, a.ref_release - 0.02))
print("  render   ", *release(me, release_me - 0.02))

fig, ax = plt.subplots(2, 2, figsize=(13, 7))
for j, (x, name) in enumerate([(ref, "RECORDING"), (me, "BLASTER")]):
    top = x[int((a.ref_press - 0.2) * fs):int((a.ref_press + a.charge_time + 0.8) * fs)] if j == 0 else x[:int((release_me + 0.3) * fs)]
    ax[0][j].specgram(top, NFFT=2048, Fs=fs, noverlap=1792, cmap="magma", vmin=-110, vmax=-20)
    ax[0][j].set_ylim(0, 2500); ax[0][j].set_title(name + ": charge (0-2.5 kHz)")
    r0 = a.ref_release - 0.1 if j == 0 else release_me - 0.1
    ax[1][j].specgram(x[int(r0 * fs):int((r0 + 1.05) * fs)], NFFT=1024, Fs=fs, noverlap=896, cmap="magma", vmin=-110, vmax=-25)
    ax[1][j].set_ylim(0, 12000); ax[1][j].set_title(name + ": release (0-12 kHz)")
plt.tight_layout(); plt.savefig(a.out, dpi=65)
print("\nwrote", a.out)
