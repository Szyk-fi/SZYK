"""The audio front ends, mirrored exactly in src/apps (each app's
feature code) and checked against each other by the apps' tests."""

import numpy as np

SR = 16000


def hann(n):
    """Periodic Hann, as the Rust `Spectrum`."""
    return (0.5 - 0.5 * np.cos(2 * np.pi * np.arange(n) / n)).astype(np.float32)


def spectrum(frames, n_fft):
    frames = np.asarray(frames, dtype=np.float32)
    w = hann(frames.shape[-1])
    return np.abs(np.fft.rfft(frames * w, n=n_fft, axis=-1)).astype(np.float32)


def mag_at(mag, hz, bin_hz):
    """Linear interpolation between bins, as `neural::mag_at`."""
    b = np.asarray(hz, dtype=np.float32) / np.float32(bin_hz)
    i = np.floor(b).astype(np.int64)
    f = (b - i).astype(np.float32)
    ok = i + 1 < mag.shape[-1]
    i = np.where(ok, i, 0)
    v = mag[..., i] * (1 - f) + mag[..., i + 1] * f
    return np.where(ok, v, 0.0).astype(np.float32)


def log_relative(x, floor):
    peak = np.maximum(x.max(axis=-1, keepdims=True), 1e-9)
    return np.maximum(np.log(np.maximum(x / peak, 1e-9)), floor).astype(np.float32)


# --- Hum: a log-frequency spectrum, 3 bins per semitone from 40 Hz -------
HUM_FRAME = 1024
HUM_FFT = 2048
HUM_BINS = 216
HUM_FREQS = (40.0 * 2.0 ** (np.arange(HUM_BINS) / 36.0)).astype(np.float32)
HUM_FLOOR = -9.0


def hum_features(frames):
    mag = spectrum(frames, HUM_FFT)
    x = mag_at(mag, HUM_FREQS, SR / HUM_FFT)
    return log_relative(x, HUM_FLOOR)


# --- Mouth Drums: 32 mel bands x 12 frames ------------------------------
MOUTH_FRAME = 256
MOUTH_HOP = 96
MOUTH_FRAMES = 12
MOUTH_MELS = 32
MOUTH_PRE = 2  # frames before the onset
MOUTH_FLOOR = -10.0
MOUTH_SAMPLES = MOUTH_FRAME + MOUTH_HOP * (MOUTH_FRAMES - 1)


def hz_to_mel(f):
    return 2595.0 * np.log10(1.0 + f / 700.0)


def mel_to_hz(m):
    return 700.0 * (10.0 ** (m / 2595.0) - 1.0)


def mel_filters(n_mels=MOUTH_MELS, n_fft=MOUTH_FRAME, lo=60.0, hi=7600.0):
    """Triangular filters on the HTK mel scale over the FFT bins."""
    pts = mel_to_hz(np.linspace(hz_to_mel(lo), hz_to_mel(hi), n_mels + 2))
    bins = np.arange(n_fft // 2 + 1) * SR / n_fft
    fb = np.zeros((n_mels, len(bins)), dtype=np.float32)
    for m in range(n_mels):
        a, c, b = pts[m], pts[m + 1], pts[m + 2]
        up = (bins - a) / (c - a)
        down = (b - bins) / (b - c)
        fb[m] = np.maximum(0, np.minimum(up, down))
    return fb


MEL_FB = mel_filters()


def mouth_features(clip):
    """clip: MOUTH_SAMPLES samples starting MOUTH_PRE hops before the
    onset. Returns [32 mels, 12 frames] (channel-major)."""
    clip = np.asarray(clip, dtype=np.float32)
    frames = np.stack([clip[..., t * MOUTH_HOP:t * MOUTH_HOP + MOUTH_FRAME] for t in range(MOUTH_FRAMES)], axis=-2)
    mag = spectrum(frames, MOUTH_FRAME)  # [..., T, 129]
    mel = mag @ MEL_FB.T  # [..., T, 32]
    peak = np.maximum(mel.max(axis=(-1, -2), keepdims=True), 1e-9)
    x = np.maximum(np.log(np.maximum(mel / peak, 1e-9)), MOUTH_FLOOR).astype(np.float32)
    return np.swapaxes(x, -1, -2)


# --- Band Mate: energy per semitone, C2 to B7 ---------------------------
CHORD_FRAME = 4096
CHORD_FFT = 4096
CHORD_LO = 36
CHORD_BINS = 72
CHORD_FLOOR = -9.0


def chord_features(frames):
    """Each semitone's energy: the sum of FFT bin powers within half a
    semitone of it, weighted by a triangle."""
    mag = spectrum(frames, CHORD_FFT)
    p = mag.astype(np.float64) ** 2
    W = chord_weights()
    e = (p @ W.T).astype(np.float32)
    return log_relative(np.sqrt(e).astype(np.float32), CHORD_FLOOR)


def chord_weights():
    bins = np.arange(CHORD_FFT // 2 + 1) * SR / CHORD_FFT
    W = np.zeros((CHORD_BINS, len(bins)))
    with np.errstate(divide="ignore"):
        semis = 12 * np.log2(np.maximum(bins, 1e-9) / 440.0) + 69
    for k in range(CHORD_BINS):
        d = np.abs(semis - (CHORD_LO + k))
        W[k] = np.maximum(0, 1 - d)
    return W


# --- Sorter: 32 mel bands x 24 frames, ~300 ms from a sample's onset -----
SORT_FRAME = 512
SORT_HOP = 192
SORT_FRAMES = 24
SORT_MELS = 32
SORT_PRE = 1  # hops before the onset
SORT_FLOOR = -10.0
SORT_SAMPLES = SORT_FRAME + SORT_HOP * (SORT_FRAMES - 1)
SORT_FB = mel_filters(SORT_MELS, SORT_FRAME)


def sort_features(clip):
    """clip: SORT_SAMPLES samples starting SORT_PRE hops before the onset.
    Returns [32 mels, 24 frames] (channel-major), relative to the loudest
    cell so it doesn't depend on the sample's level."""
    clip = np.asarray(clip, dtype=np.float32)
    frames = np.stack([clip[..., t * SORT_HOP:t * SORT_HOP + SORT_FRAME] for t in range(SORT_FRAMES)], axis=-2)
    mag = spectrum(frames, SORT_FRAME)
    mel = mag @ SORT_FB.T
    peak = np.maximum(mel.max(axis=(-1, -2), keepdims=True), 1e-9)
    x = np.maximum(np.log(np.maximum(mel / peak, 1e-9)), SORT_FLOOR).astype(np.float32)
    return np.swapaxes(x, -1, -2)
