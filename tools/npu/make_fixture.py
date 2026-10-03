"""A small random network using every layer type, so the Rust runtime's
bit-exactness is checked on all of them (src/apps/neural.rs tests)."""

import numpy as np
import torch

import pmxn

torch.manual_seed(0)
rng = np.random.default_rng(0)
spec = [
    ("conv", 3, 8, 5, 1, 2, True),
    ("pool", 2),
    ("conv", 8, 12, 3, 2, 1, True),
    ("conv", 12, 6, 3, 1, 1, False),
    ("gap",),
    ("dense", 6, 10, True),
    ("dense", 10, 4, False),
]
spec2 = [
    ("conv", 3, 4, 3, 1, 0, True),
    ("flatten",),
    ("dense", 4 * 14, 5, False),
]
for name, sp in (("selftest", spec), ("selftest_flat", spec2)):
    net = pmxn.Net(sp, (3, 16))
    calib = rng.normal(size=(256, 3, 16)).astype(np.float32)
    q = pmxn.quantise(net, calib)
    tests = [rng.normal(size=(3, 16)).astype(np.float32) * s for s in (0.5, 1.0, 3.0)]
    pmxn.export(net, q, name, "fixtures", tests)
    print(name, "ok")
