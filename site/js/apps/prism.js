/* Prism: a multitap delay. Time sets the spacing of up to eight taps, Repeats how fast they fade, Shape how
   they bunch together. The panel draws the real tap distribution. */
(() => {
  'use strict';
  const R = window.ROVE, A = R.audio, { ctl, fmt, rgba } = R, S = R.slint;
  const PAL = S.pal('#080410', '#EFE3FF', '#B565FF', '#6B597B');
  const TAPS = 8;
  R.registerApp({
    id: 'prism', name: 'Prism', category: 'effect',
    desc: 'A multitap delay: up to eight taps with a spacing curve, feedback, a wobble and a dark filter.',
    create: R.effectApp({
      name: 'Prism', pal: PAL, crumb: 'MULTITAP DELAY',
      kit(inst) {
        const c = [
          ctl.f('Time', 40, 600, 300, { fmt: (v) => `${Math.round(v)} ms`, log: true }), ctl.f('Repeats', 0, 0.95, 0.35, { fmt: fmt.pct }), ctl.f('Shape', 0, 1, 0.4, { fmt: fmt.d2 }), ctl.f('Mix', 0, 1, 0.5, { fmt: fmt.d2 }),
          ctl.int('Taps', 2, 8, 4),
        ];
        inst.c = c; inst.m = {};
        const h = (k) => ({ label: k.label, control: 0, to: 0, hold: (d) => { inst.m[k.key] = d; } });
        const throws = [['FEEDBACK', 'fb'], ['WET', 'wet'], ['SPACE', 'space'], ['DARK', 'dark'], ['SHORT', 'short'], ['LONG', 'long'], ['8 TAPS', 'eight'], ['WOBBLE', 'wobble']].map(([label, key]) => h({ label, key }));
        return { controls: c, hero: [[0, 1], [2, 3]], throws, routes: { sx: 0, sy: 1, hl: 3 } };
      },
      line: (inst) => 'Play',
      build(inst) {
        const c = A.ctx, inp = A.gain(1), out = A.gain(1), dry = A.gain(0.5), wet = A.gain(0.5), fbk = A.gain(0), lp = A.filt('lowpass', 8000, 0.5);
        inp.connect(dry); dry.connect(out); wet.connect(out);
        const taps = Array.from({ length: TAPS }, () => { const d = c.createDelay(2.5), g = A.gain(0); inp.connect(d); d.connect(g); g.connect(wet); return { d, g }; });
        const sum = A.gain(1); taps.forEach((t) => t.g.connect(sum)); sum.connect(lp); lp.connect(fbk); fbk.connect(taps[0].d);
        const lfo = A.osc('sine', 0.6, A.T()), lg = A.gain(0); lfo.connect(lg); taps.forEach((t) => lg.connect(t.d.delayTime)); lfo.start();
        inst.px = { taps, dry, wet, fbk, lp, lg }; return { in: inp, out };
      },
      update(inst) {
        const c = inst.c, p = inst.px, m = inst.m, t = A.T();
        const time = (c[0].value / 1000) * (m.short ? 0.35 : 1) * (m.long ? 2.4 : 1), rep = Math.min(0.95, c[1].value + (m.fb ? 0.3 : 0)), n = m.eight ? 8 : c[4].value, shape = c[2].value;
        inst.tapInfo = [];
        p.taps.forEach((tp, i) => {
          const pos = (i + 1) ** (1 + shape * 0.9) * time / (n ** (shape * 0.9)) * 1.0 + 0.001;      // shape bunches later taps
          tp.d.delayTime.setTargetAtTime(Math.min(2.4, pos), t, 0.05); tp.g.gain.setTargetAtTime(i < n ? rep ** (i * 0.8 + 0.2) * 0.9 : 0, t, 0.05);
          if (i < n) inst.tapInfo.push([pos, rep ** (i * 0.8 + 0.2)]);
        });
        p.fbk.gain.setTargetAtTime(m.fb ? 0.55 : 0.12, t, 0.05); p.lp.frequency.setTargetAtTime(m.dark ? 1200 : 8000, t, 0.05); p.lg.gain.setTargetAtTime(m.wobble ? 0.004 : 0, t, 0.05);
        const wet = Math.min(1, c[3].value + (m.wet ? 0.4 : 0)); p.wet.gain.setTargetAtTime(wet, t, 0.05); p.dry.gain.setTargetAtTime(1 - wet * 0.6, t, 0.05); inst.ch.setSend(m.space ? 0.7 : 0.1);
      },
      drawRight(fb, inst, pal) {
        S.card(fb, pal, 'DELAY / TAP DISTRIBUTION');
        const info = inst.tapInfo || [], longest = Math.max(0.1, ...info.map((i) => i[0]));
        info.forEach(([pos, g]) => { const x = 326 + (pos / longest) * 270, h = 190 * g; fb.rect(x, 280 - h - 24, 2.5, h, pal.accent); });
        fb.rect(326, 256, 286, 1, rgba(pal.ink, 0.2));
        fb.ttf(`${info.length} taps — longest ${Math.round(longest * 1000)} ms`, 326, 282, 12, pal.accent);
      },
    }),
  });
})();
