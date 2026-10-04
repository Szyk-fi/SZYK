"""Conductor: hand gestures over Portamax's two depth sensors.

Input: the last 0.8 s of both sensors (2 channels x 48 frames at 60 Hz;
0 = no hand, rising to 1 as a hand comes close). Output: 8 classes --
nothing, swipe right, swipe left, push (both hands in), wave over the
left or right sensor, tap over the left or right sensor.

The training data is synthesized sensor traces: each gesture as bumps
with random height, width, timing, speed and lag between the sensors,
placed so it ends near the newest frame (the app classifies a sliding
window), plus "nothing" traces made of the things a player does that
mustn't trigger anything -- holding a hand still to play, slow drifts,
a hand slowly arriving or leaving, sensor noise and dropouts.
"""

import sys
import numpy as np
import torch

import pmxn

N = 48
CLASSES = ["nothing", "swipe right", "swipe left", "push", "wave left", "wave right", "tap left", "tap right"]
SPEC = [
    ("conv", 2, 16, 5, 1, 2, True),
    ("pool", 2),
    ("conv", 16, 32, 5, 1, 2, True),
    ("pool", 2),
    ("conv", 32, 32, 3, 1, 1, True),
    ("flatten",),
    ("dense", 32 * 12, 32, True),
    ("dense", 32, len(CLASSES), False),
]
t = np.arange(N, dtype=np.float32)


def bump(center, width, height):
    """A hand passing over a sensor: a smooth rise and fall."""
    return height * np.exp(-0.5 * ((t - center) / max(width, 0.5)) ** 2)


def plateau(start, rise, length, height):
    up = 1 / (1 + np.exp(-(t - start) / max(rise, 0.3)))
    down = 1 / (1 + np.exp((t - start - length) / max(rise, 0.3)))
    return height * up * down


def one(rng, k, stale=False, partial=False):
    x = np.zeros((2, N), dtype=np.float32)
    # The gesture finishes near the newest frame -- or, for a "stale"
    # example (labelled nothing), well before it: one already recognised
    # and now sliding out of the window.
    # A "partial" one (also nothing) is still under way: a swipe whose
    # second half hasn't happened yet looks like a tap until it does.
    end = rng.uniform(-4, 28) if stale else rng.uniform(48, 62) if partial else rng.uniform(34, 46)
    if k in (6, 7) and not stale:
        # A tap only counts once the other sensor has stayed quiet long
        # enough that it can't be the start of a swipe or a wave; until
        # then (a "partial" tap) it's nothing yet.
        end = rng.uniform(34, 50) if partial else rng.uniform(16, 32)
    h = lambda: rng.uniform(0.25, 1.0)
    if k == 0:
        mode = rng.integers(0, 5)
        if mode == 1:  # holding still (playing)
            for c in range(2):
                if rng.random() < 0.7:
                    x[c] = rng.uniform(0.1, 0.95) + 0.03 * np.sin(t / rng.uniform(4, 20) + rng.uniform(0, 6))
        elif mode == 2:  # slow drift
            for c in range(2):
                a, b = rng.uniform(0, 1, 2)
                x[c] = a + (b - a) * t / N
        elif mode == 3:  # slowly arriving or leaving
            c = rng.integers(0, 2)
            x[c] = plateau(rng.uniform(-10, 40), rng.uniform(5, 12), 100, h()) if rng.random() < 0.5 else plateau(-50, rng.uniform(5, 12), rng.uniform(55, 90), h())
        elif mode == 4:  # a gesture that finished long ago, half out of the window
            c = rng.integers(0, 2)
            x[c] = bump(rng.uniform(-6, 6), rng.uniform(2, 5), h())
    elif k in (1, 2):
        first, second = (0, 1) if k == 1 else (1, 0)
        w = rng.uniform(2, 6)
        lag = rng.uniform(3, 12)
        hh = h()
        x[second] = bump(end - w, w, hh * rng.uniform(0.7, 1.3))
        x[first] = bump(end - w - lag, w, hh * rng.uniform(0.7, 1.3))
    elif k == 3:
        start = end - rng.uniform(6, 20)
        rise = rng.uniform(0.5, 2)
        for c in range(2):
            x[c] = plateau(start + rng.uniform(-1.5, 1.5), rise, rng.uniform(4, 60), h())
    elif k in (4, 5):
        c = 0 if k == 4 else 1
        period = rng.uniform(6, 14)
        n = rng.integers(2, 4)
        hh = h()
        for j in range(n):
            x[c] += bump(end - period * (n - 1 - j) - 2, period / 4, hh * rng.uniform(0.7, 1.0))
        x[c] = np.minimum(x[c], 1.0)
    else:
        c = 0 if k == 6 else 1
        w = rng.uniform(1.2, 3.5)
        x[c] = bump(end - w, w, h())
        # The other hand may be resting still over its sensor.
        if rng.random() < 0.3:
            x[1 - c] = rng.uniform(0.1, 0.8)
    # Sensor reality: noise, quantisation, occasional dropouts.
    x += rng.normal(0, rng.uniform(0.003, 0.03), size=x.shape)
    if rng.random() < 0.2:
        c = rng.integers(0, 2)
        i = rng.integers(0, N)
        x[c, i:i + rng.integers(1, 3)] = 0
    steps = rng.choice([0, 64, 256])
    if steps:
        x = np.round(x * steps) / steps
    return np.clip(x, 0, 1).astype(np.float32)


def make(rng, per_class):
    xs = [one(rng, k) for k in range(len(CLASSES)) for _ in range(per_class)]
    ys = [k for k in range(len(CLASSES)) for _ in range(per_class)]
    return np.stack(xs), np.array(ys)


def main():
    rng = np.random.default_rng(3)
    torch.manual_seed(3)
    per = int(sys.argv[1]) if len(sys.argv) > 1 else 6000
    x, y = make(rng, per)
    # "Nothing" is most of what the sensors see: weight it up.
    extra = [one(rng, 0) for _ in range(per * 2)] + [one(rng, int(rng.integers(1, len(CLASSES))), stale=True) for _ in range(per * 2)] + [one(rng, int(rng.choice([1, 2, 4, 5, 6, 7])), partial=True) for _ in range(per * 2)]
    x = np.concatenate([x, np.stack(extra)])
    y = np.concatenate([y, np.zeros(per * 6, dtype=y.dtype)])
    vx, vy = make(np.random.default_rng(9), 400)
    net = pmxn.Net(SPEC, (2, N))

    def val(net):
        with torch.no_grad():
            p = net(torch.tensor(vx)).argmax(1).numpy()
        conf = np.zeros((len(CLASSES), len(CLASSES)), dtype=int)
        for a, b in zip(vy, p):
            conf[a, b] += 1
        false_alarms = (p[vy == 0] != 0).mean()
        return f"acc {(p == vy).mean() * 100:.1f}%  false triggers {false_alarms * 100:.1f}%"

    pmxn.train(net, x, y, epochs=int(sys.argv[2]) if len(sys.argv) > 2 else 25, lr=3e-3, val=val)
    q = pmxn.quantise(net, x[np.random.default_rng(5).choice(len(x), 2000, replace=False)])
    p8 = np.array([pmxn.run_int8(q, v).argmax() for v in vx])
    print(f"int8 acc {(p8 == vy).mean() * 100:.1f}%")
    conf = np.zeros((len(CLASSES), len(CLASSES)), dtype=int)
    for a, b in zip(vy, p8):
        conf[a, b] += 1
    print(conf)
    macs = pmxn.export(net, q, "gesture", "../../assets/npu", [vx[0], vx[401], vx[1300]], meta=dict(classes=CLASSES, frames=N))
    print("MACs", macs)


if __name__ == "__main__":
    main()
