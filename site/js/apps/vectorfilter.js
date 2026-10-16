/* Vector Filter: a resonant multimode filter with drive, drawn as a space you move through: cutoff along X,
   resonance up Y, drive into the depth. */
(() => {
  'use strict';
  const R = window.ROVE, A = R.audio, { ctl, fmt, rgba } = R, S = R.slint;
  const PAL = S.pal('#101E25', '#E4F0E8', '#79E2CF', '#729D9E');
  const TYPES = ['Low pass', 'Band pass', 'High pass'], BQ = ['lowpass', 'bandpass', 'highpass'];
  R.registerApp({
    id: 'vector_filter', name: 'Vector Filter', category: 'effect',
    desc: 'A resonant multimode filter with drive. Cutoff, resonance and drive are three axes of one space.',
    create: R.effectApp({
      name: 'Vector Filter', title: 'Vector Filter', pal: PAL, crumb: 'FILTER', colW: 222,
      kit(inst) {
        const c = [
          ctl.f('Cutoff', 40, 16000, 2215, { fmt: (v) => `${Math.round(v)} Hz`, log: true }), ctl.f('Resonance', 0.5, 20, 4.85, { fmt: (v) => `Q ${v.toFixed(2)}`, log: true }), ctl.f('Drive', 1, 12, 5.2, { fmt: (v) => `${v.toFixed(1)}×`, log: true }), ctl.f('Wet', 0, 1, 1, { fmt: fmt.pct }),
          ctl.choice('Type', TYPES, 0),
        ];
        inst.c = c; inst.m = {};
        const h = (label, key) => ({ label, control: 0, to: 0, hold: (d) => { inst.m[key] = d; } });
        return { controls: c, hero: [[0, 1], [2, 3]], throws: [h('OPEN', 'open'), h('CLOSE', 'close'), h('SCREAM', 'scream'), h('DRIVE', 'drive'), h('DRY', 'dry'), h('BAND', 'band'), h('HIGH', 'high'), h('BYPASS', 'bypass')], routes: { sx: 0, sy: 1, hl: 2, hr: 3 } };
      },
      build(inst) {
        const c = A.ctx, inp = A.gain(1), out = A.gain(1), f = A.filt('lowpass', 2000, 4), pre = A.shaper(5, false), wet = A.gain(1), dry = A.gain(0), post = A.gain(0.7);
        inp.connect(pre); pre.connect(f); f.connect(post); post.connect(wet); wet.connect(out); inp.connect(dry); dry.connect(out); inst.px = { f, wet, dry, pre, post }; return { in: inp, out };
      },
      update(inst) {
        const c = inst.c, m = inst.m, p = inst.px, t = A.T(), type = m.band ? 1 : m.high ? 2 : c[4].value;
        const cut = m.open ? 16000 : m.close ? 60 : c[0].value, q = m.scream ? 18 : c[1].value, drive = m.drive ? 12 : m.scream ? 9 : c[2].value;
        p.f.type = BQ[type]; p.f.frequency.setTargetAtTime(cut, t, 0.02); p.f.Q.setTargetAtTime(q, t, 0.02);
        if (Math.abs((p.driveNow || 0) - drive) > 0.3) { p.driveNow = drive; p.pre.curve = A.shaper(drive, false).curve; }
        const w = m.dry ? 0 : c[3].value; p.wet.gain.setTargetAtTime(m.bypass ? 0 : w, t, 0.02); p.dry.gain.setTargetAtTime(m.bypass ? 1 : 1 - w, t, 0.02); inst.eff = { cut, q, drive, type };
      },
      drawRight(fb, inst, pal) {
        const e = inst.eff || { cut: 2215, q: 4.85, drive: 5.2, type: 0 }, x0 = 262, y0 = 62, w = 270, h = 150, dx = 36, dy = -18;
        fb.rr(250, 54, 372, 200, 6, rgba(pal.ink, 0.03)); fb.ttf('RESONANT SPACE', 262, 68, 12, pal.accent, { spacing: 1 }); fb.ttf(TYPES[e.type], 612, 68, 12, rgba(pal.ink, 0.8), { align: 'right' });
        const box = (ox, oy, c) => { fb.g.strokeStyle = c; fb.g.lineWidth = 1.3; fb.g.strokeRect(x0 + ox, y0 + 24 + oy, w, h); };
        box(0, 0, rgba(pal.accent, 0.8)); box(dx, dy, rgba(pal.accent, 0.35));
        [[0, 0, dx, dy], [w, 0, w + dx, dy], [0, h, dx, h + dy], [w, h, w + dx, h + dy]].forEach(([a, b, c2, d]) => { fb.g.strokeStyle = rgba(pal.accent, 0.5); fb.g.beginPath(); fb.g.moveTo(x0 + a, y0 + 24 + b); fb.g.lineTo(x0 + c2, y0 + 24 + d); fb.g.stroke(); });
        const resp = (f) => { const r = f / e.cut; const base = e.type === 0 ? 1 / Math.sqrt(1 + r ** 4) : e.type === 2 ? r ** 2 / Math.sqrt(1 + r ** 4) : r / Math.sqrt(1 + r ** 4) * 1.4; return base * (1 + (e.q > 1 ? Math.exp(-((Math.log(r)) ** 2) * 14) * (e.q * 0.12) : 0)); };
        for (let k = 0; k < 24; k++) {
          const s = k / 23, ox = dx * s, oy = dy * s; fb.g.strokeStyle = rgba(pal.accent, 0.25 + s * 0.55); fb.g.lineWidth = 1.2; fb.g.beginPath();
          for (let i = 0; i <= 60; i++) { const fr = 40 * (16000 / 40) ** (i / 60), v = Math.min(1.5, resp(fr)) / 1.5, px = x0 + ox + (i / 60) * w, py = y0 + 24 + oy + h - v * h * (0.2 + e.drive / 16) * (1 + 0.15 * Math.sin(k * 0.9 + i * 0.35)); i ? fb.g.lineTo(px, py) : fb.g.moveTo(px, py); }
          fb.g.stroke();
        }
        const cx = x0 + (Math.log(e.cut / 40) / Math.log(400)) * w + dx * 0.5, cy = y0 + 24 + h - (Math.log(e.q / 0.5) / Math.log(40)) * h + dy * 0.5; fb.disc(cx - 5, cy - 5, 10, pal.accent);
        fb.ttf('X CUTOFF →', 262, 258 - 2, 12, pal.accent); fb.ttf('Y / Q', 258, 96, 12, pal.accent);
        fb.rr(262, 270, 360, 22, 4, rgba(pal.accent, 0.25)); fb.ttf(`Z / DRIVE  ${Math.round(Math.min(1, e.drive / 12) * 100)}%  ⇔`, 442, 285, 12, pal.ink, { align: 'center' });
      },
    }),
  });
})();
