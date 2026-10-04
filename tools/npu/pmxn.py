"""Build, quantise and export the small networks Portamax runs on the
STM32N6's Neural-ART NPU.

A network is a list of layers from the set the NPU accelerates and the
Portamax runtime (src/apps/neural.rs) implements:

    ("conv", cin, cout, k, stride, pad, relu)   1-D convolution over [C, L]
    ("pool", k)                                  max pool, stride k
    ("flatten",)                                 [C, L] -> [C*L], channel-major
    ("dense", nin, nout, relu)                   fully connected
    ("gap",)                                     global average pool over L

Quantisation is the TFLite/ONNX-QDQ scheme ST Edge AI also uses: int8
activations with a scale and zero point (asymmetric), int8 weights with a
scale per output channel (symmetric), int32 biases, int32 accumulation,
and a float32 multiplier to requantise each output. `run_int8` here is the
reference: the Rust runtime matches it bit for bit (checked by the test
vectors each export writes), so what the sim plays is what the chip would
compute.

Export writes:
    <name>.pmxn      the int8 model for the Portamax runtime
    <name>.onnx      the float model, for ST Edge AI (`stedgeai generate`)
    <name>.test.json a few inputs and the exact int8 outputs expected
"""

import json
import struct

import numpy as np
import torch
import torch.nn as nn

MAGIC = b"PMXN"
VERSION = 1


class Net(nn.Module):
    def __init__(self, spec, in_shape):
        super().__init__()
        self.spec = spec
        self.in_shape = tuple(in_shape)
        mods = []
        for s in spec:
            if s[0] == "conv":
                _, cin, cout, k, stride, pad, _relu = s
                mods.append(nn.Conv1d(cin, cout, k, stride=stride, padding=pad))
            elif s[0] == "dense":
                _, nin, nout, _relu = s
                mods.append(nn.Linear(nin, nout))
            else:
                mods.append(None)
        self.mods = nn.ModuleList([m for m in mods if m is not None])
        self._map = []
        j = 0
        for m in mods:
            if m is None:
                self._map.append(None)
            else:
                self._map.append(j)
                j += 1

    def forward(self, x, taps=None):
        for s, j in zip(self.spec, self._map):
            if s[0] == "conv":
                x = self.mods[j](x)
                if s[6]:
                    x = torch.relu(x)
            elif s[0] == "dense":
                if x.dim() == 3:
                    x = x.flatten(1)
                x = self.mods[j](x)
                if s[3]:
                    x = torch.relu(x)
            elif s[0] == "pool":
                x = nn.functional.max_pool1d(x, s[1])
            elif s[0] == "flatten":
                x = x.flatten(1)
            elif s[0] == "gap":
                x = x.mean(dim=2)
            if taps is not None:
                taps.append(x.detach())
        return x


def macs(spec, in_shape):
    c, length = in_shape
    total = 0
    for s in spec:
        if s[0] == "conv":
            _, cin, cout, k, stride, pad, _ = s
            length = (length + 2 * pad - k) // stride + 1
            total += length * cout * cin * k
            c = cout
        elif s[0] == "pool":
            length //= s[1]
        elif s[0] == "flatten":
            c, length = c * length, 1
        elif s[0] == "dense":
            total += s[1] * s[2]
            c, length = s[2], 1
        elif s[0] == "gap":
            length = 1
    return total


def f32(x):
    return np.float32(x)


def act_qparams(lo, hi):
    lo = min(float(lo), 0.0)
    hi = max(float(hi), lo + 1e-6)
    scale = f32((hi - lo) / 255.0)
    zp = int(np.clip(np.floor(-128.0 - lo / float(scale) + 0.5), -128, 127))
    return scale, zp


def rnd(x):
    """Round half up, in float32 -- the same as the Rust runtime."""
    return np.floor(x.astype(np.float32) + np.float32(0.5))


def quantize_input(x, scale, zp):
    q = rnd(x.astype(np.float32) / np.float32(scale)) + zp
    return np.clip(q, -128, 127).astype(np.int8)


def quantise(net, calib):
    """Post-training quantisation: per-layer activation ranges from the
    calibration set (0.01% / 99.99% quantiles, so one outlier can't waste
    the int8 range), per-channel weight scales."""
    net.eval()
    with torch.no_grad():
        x = torch.tensor(calib, dtype=torch.float32)
        taps = []
        net(x, taps)
    xin = calib.reshape(-1)
    in_scale, in_zp = act_qparams(np.quantile(xin, 0.0001), np.quantile(xin, 0.9999))
    layers = []
    cur_scale, cur_zp = in_scale, in_zp
    for s, j, t in zip(net.spec, net._map, taps):
        t = t.numpy().reshape(-1)
        if s[0] in ("conv", "dense"):
            m = net.mods[j]
            w = m.weight.detach().numpy().astype(np.float32)
            b = m.bias.detach().numpy().astype(np.float32)
            cout = w.shape[0]
            wmax = np.abs(w.reshape(cout, -1)).max(axis=1)
            w_scale = np.maximum(wmax / 127.0, 1e-12).astype(np.float32)
            wq = np.clip(rnd(w / w_scale.reshape((cout,) + (1,) * (w.ndim - 1))), -127, 127).astype(np.int8)
            relu = s[6] if s[0] == "conv" else s[3]
            lo = 0.0 if relu else np.quantile(t, 0.0001)
            out_scale, out_zp = act_qparams(lo, np.quantile(t, 0.9999))
            bias = np.round(b.astype(np.float64) / (float(cur_scale) * w_scale.astype(np.float64))).astype(np.int64)
            bias = np.clip(bias, -(2**31), 2**31 - 1).astype(np.int32)
            mult = (np.float32(cur_scale) * w_scale / np.float32(out_scale)).astype(np.float32)
            layers.append(dict(spec=s, w=wq, bias=bias, mult=mult, in_scale=cur_scale, in_zp=cur_zp, out_scale=out_scale, out_zp=out_zp, relu=relu))
            cur_scale, cur_zp = out_scale, out_zp
        elif s[0] == "gap":
            out_scale, out_zp = act_qparams(np.quantile(t, 0.0001), np.quantile(t, 0.9999))
            layers.append(dict(spec=s, in_scale=cur_scale, in_zp=cur_zp, out_scale=out_scale, out_zp=out_zp))
            cur_scale, cur_zp = out_scale, out_zp
        else:
            layers.append(dict(spec=s, in_scale=cur_scale, in_zp=cur_zp, out_scale=cur_scale, out_zp=cur_zp))
    return dict(in_shape=net.in_shape, in_scale=in_scale, in_zp=in_zp, layers=layers, out_scale=cur_scale, out_zp=cur_zp)


def run_int8(q, x):
    """The reference int8 inference for one input `x` (float, in_shape)."""
    a = quantize_input(np.asarray(x, dtype=np.float32).reshape(q["in_shape"]), q["in_scale"], q["in_zp"]).astype(np.int32)
    for L in q["layers"]:
        s = L["spec"]
        if s[0] == "conv":
            _, cin, cout, k, stride, pad, relu = s
            zp = L["in_zp"]
            length = a.shape[1]
            padded = np.full((cin, length + 2 * pad), zp, dtype=np.int32)
            padded[:, pad:pad + length] = a
            out_len = (length + 2 * pad - k) // stride + 1
            cols = np.stack([padded[:, t * stride:t * stride + k] for t in range(out_len)], axis=0)  # [T, cin, k]
            acc = np.einsum("tck,ock->ot", cols - zp, L["w"].astype(np.int32)).astype(np.int64)
            acc = (acc + L["bias"].reshape(-1, 1)).astype(np.int32)
            a = requant(acc, L["mult"].reshape(-1, 1), L["out_zp"], relu)
        elif s[0] == "dense":
            v = a.reshape(-1) - L["in_zp"]
            acc = (L["w"].astype(np.int32) @ v).astype(np.int64) + L["bias"]
            a = requant(acc.astype(np.int32), L["mult"], L["out_zp"], s[3]).reshape(-1, 1)
        elif s[0] == "pool":
            k = s[1]
            n = a.shape[1] // k
            a = a[:, :n * k].reshape(a.shape[0], n, k).max(axis=2)
        elif s[0] == "flatten":
            a = a.reshape(-1, 1)
        elif s[0] == "gap":
            length = a.shape[1]
            acc = (a - L["in_zp"]).sum(axis=1).astype(np.int32)
            m = gap_mult(L, length)
            a = np.clip(rnd(acc.astype(np.float32) * m) + L["out_zp"], -128, 127).astype(np.int32).reshape(-1, 1)
    return a.reshape(-1).astype(np.int8)


def gap_mult(L, length):
    return np.float32(np.float32(L["in_scale"]) / (np.float32(length) * np.float32(L["out_scale"])))


def requant(acc, mult, zp, relu):
    v = rnd(acc.astype(np.float32) * mult) + zp
    lo = max(-128, zp) if relu else -128
    return np.clip(v, lo, 127).astype(np.int32)


def dequant(q, out):
    return (out.astype(np.float32) - q["out_zp"]) * np.float32(q["out_scale"])


def export(net, q, name, outdir, tests, meta=None):
    """Writes <name>.pmxn, <name>.onnx and <name>.test.json."""
    blob = bytearray()
    layers = []
    for L in q["layers"]:
        s = L["spec"]
        d = dict(type=s[0])
        if s[0] == "conv":
            _, cin, cout, k, stride, pad, relu = s
            d.update(cin=cin, cout=cout, k=k, stride=stride, pad=pad, relu=bool(relu))
        elif s[0] == "dense":
            d.update(nin=s[1], nout=s[2], relu=bool(s[3]))
        elif s[0] == "pool":
            d.update(k=s[1])
        elif s[0] == "gap":
            d["mult_per_len"] = True
        if "w" in L:
            d["w_off"] = len(blob)
            d["w_len"] = int(L["w"].size)
            blob += L["w"].astype(np.int8).tobytes()
            while len(blob) % 4:
                blob += b"\0"
            d["b_off"] = len(blob)
            blob += L["bias"].astype("<i4").tobytes()
            d["mult"] = [float(m) for m in L["mult"]]
        d.update(in_scale=float(L["in_scale"]), in_zp=int(L["in_zp"]), out_scale=float(L["out_scale"]), out_zp=int(L["out_zp"]))
        layers.append(d)
    header = dict(name=name, version=VERSION, input=dict(shape=list(q["in_shape"]), scale=float(q["in_scale"]), zp=int(q["in_zp"])), output=dict(scale=float(q["out_scale"]), zp=int(q["out_zp"])), macs=int(macs(net.spec, q["in_shape"])), layers=layers, meta=meta or {})
    hj = json.dumps(header, separators=(",", ":")).encode()
    with open(f"{outdir}/{name}.pmxn", "wb") as f:
        f.write(MAGIC + struct.pack("<II", VERSION, len(hj)) + hj + bytes(blob))
    net.eval()
    dummy = torch.zeros((1,) + tuple(q["in_shape"]), dtype=torch.float32)
    torch.onnx.export(net, (dummy,), f"{outdir}/{name}.onnx", input_names=["input"], output_names=["output"], opset_version=17, dynamo=False)
    cases = []
    for x in tests:
        cases.append(dict(input=[float(v) for v in np.asarray(x, dtype=np.float32).reshape(-1)], output=[int(v) for v in run_int8(q, x)]))
    with open(f"{outdir}/{name}.test.json", "w") as f:
        json.dump(dict(cases=cases), f)
    return header["macs"]


def train(net, x, y, epochs, lr=2e-3, batch=256, loss_fn=None, val=None, log=print, weight_decay=1e-4):
    """Plain Adam training with a cosine learning-rate decay."""
    opt = torch.optim.AdamW(net.parameters(), lr=lr, weight_decay=weight_decay)
    xt = torch.tensor(x, dtype=torch.float32)
    yt = torch.tensor(y)
    n = len(xt)
    steps = epochs * ((n + batch - 1) // batch)
    sched = torch.optim.lr_scheduler.OneCycleLR(opt, max_lr=lr, total_steps=steps)
    loss_fn = loss_fn or nn.CrossEntropyLoss()
    for ep in range(epochs):
        net.train()
        perm = torch.randperm(n)
        total = 0.0
        for i in range(0, n, batch):
            idx = perm[i:i + batch]
            opt.zero_grad()
            loss = loss_fn(net(xt[idx]), yt[idx])
            loss.backward()
            opt.step()
            sched.step()
            total += float(loss.detach()) * len(idx)
        msg = f"epoch {ep + 1}/{epochs} loss {total / n:.4f}"
        if val is not None:
            msg += " " + val(net)
        log(msg)
    net.eval()
    return net
