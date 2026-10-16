/* Squeeze: a compressor with a gain-reduction meter. The source is the demo loop while it is open. */
(() => {
  'use strict';
  const R = window.ROVE, A = R.audio, { ctl, fmt, rgba } = R, S = R.slint;
  const PAL = S.pal('#14191F', '#E7EDF4', '#A5BCE9', '#8994AA');
  const PRESET = {   // [threshold dB, ratio, attack ms, release ms, makeup dB]
    SLAM: [-30, 12, 3, 80, 12], LIMIT: [-8, 20, 1, 60, 3], PUMP: [-24, 6, 25, 300, 8], SMOOTH: [-16, 2.5, 20, 250, 3], PUNCH: [-20, 5, 35, 120, 5], NYC: [-34, 8, 5, 200, 14],
  };
  R.registerApp({
    id: 'squeeze', name: 'Squeeze', category: 'effect',
    desc: 'A compressor with attack, release, makeup and a wet/dry mix, and a live gain-reduction meter.',
    create: R.effectApp({
      name: 'Squeeze', pal: PAL, crumb: 'COMPRESSOR',
      kit(inst) {
        const c = [
          ctl.f('Threshold', -60, 0, -18, { fmt: fmt.db }), ctl.f('Ratio', 1, 20, 4, { fmt: (v) => `${v.toFixed(1)}:1` }), ctl.f('Attack', 1, 100, 10, { fmt: (v) => `${v.toFixed(1)} ms`, log: true }), ctl.f('Release', 20, 1000, 150, { fmt: (v) => `${Math.round(v)} ms`, log: true }),
          ctl.f('Makeup', 0, 24, 6, { fmt: (v) => `+${v.toFixed(1)} dB` }), ctl.f('Mix', 0, 1, 1, { fmt: (v) => `${Math.round(v * 100)}% wet` }),
        ];
        const set = (name) => (down) => { const p = PRESET[name], idx = [0, 1, 2, 3, 4]; if (down) { inst.saved = c.map((x) => x.base); idx.forEach((i) => { c[i].base = p[i]; }); } else if (inst.saved) { c.forEach((x, i) => { x.base = inst.saved[i]; }); } };
        inst.c = c; inst.gr = 0;
        const names = ['SLAM', 'LIMIT', 'PUMP', 'SMOOTH', 'PUNCH', 'NYC', 'DRY', 'BYPASS'];
        const throws = names.map((n) => ({ label: n, control: 0, to: 0, hold: n === 'DRY' ? (d) => { c[5].base = d ? 0 : 1; } : n === 'BYPASS' ? (d) => { inst.bypass = d; inst.dry && inst.dry.gain.setTargetAtTime(d ? 1 : 0, A.T(), 0.01); inst.wetOut.gain.setTargetAtTime(d ? 0 : 1, A.T(), 0.01); } : set(n) }));
        return { controls: c, hero: [[0, 1], [2, 3]], throws, routes: { sx: 0, sy: 1, hl: 4, hr: 5 } };
      },
      build(inst) {
        const c = A.ctx, comp = c.createDynamicsCompressor(), mk = A.gain(1), wet = A.gain(1), dry = A.gain(0), inp = A.gain(1), out = A.gain(1);
        comp.knee.value = 6; inp.connect(comp); comp.connect(mk); mk.connect(wet); wet.connect(out); inp.connect(dry); dry.connect(out);
        inst.comp = comp; inst.mk = mk; inst.wet = wet; inst.mdry = dry;
        inst.outMeter = c.createAnalyser(); inst.outMeter.fftSize = 512; out.connect(inst.outMeter); inst._ob = new Float32Array(512);
        return { in: inp, out };
      },
      update(inst) {
        const c = inst.c, t = A.T();
        inst.comp.threshold.setTargetAtTime(c[0].value, t, 0.02); inst.comp.ratio.setTargetAtTime(c[1].value, t, 0.02); inst.comp.attack.setTargetAtTime(c[2].value / 1000, t, 0.02); inst.comp.release.setTargetAtTime(c[3].value / 1000, t, 0.02);
        inst.mk.gain.setTargetAtTime(10 ** (c[4].value / 20), t, 0.02); inst.wet.gain.setTargetAtTime(c[5].value, t, 0.02); inst.mdry.gain.setTargetAtTime(1 - c[5].value, t, 0.02);
        inst.gr += (-inst.comp.reduction - inst.gr) * 0.3;
      },
      drawRight(fb, inst, pal) {
        S.card(fb, pal, 'COMPRESSOR / GAIN REDUCTION');
        fb.ttf(`GR ${inst.gr.toFixed(1)} dB`, 326, 90, 16, pal.ink, { family: 'sans', weight: 700 });
        S.meter(fb, pal, 326, 98, 286, 4, inst.gr / 24);
        const lvl = [['IN', R.inLevel(inst) * 3], ['GR', inst.gr / 18], ['OUT', inst.level * 3]];
        lvl.forEach(([n, v], i) => { const y = 128 + i * 18; fb.ttf(n, 326, y, 12, rgba(pal.ink, 0.7)); for (let k = 0; k < 28; k++) fb.rect(358 + k * 9, y - 6, 5, 2, k / 28 < Math.min(1, v) ? pal.accent : rgba(pal.ink, 0.25)); });
        [['threshold', inst.c[0].text], ['ratio', inst.c[1].text], ['attack', inst.c[2].text], ['release', inst.c[3].text], ['makeup', inst.c[4].text], ['mix', inst.c[5].text]].forEach(([k, v], i) => { fb.ttf(k, 326, 200 + i * 14, 12, rgba(pal.ink, 0.72)); fb.ttf(v, 400, 200 + i * 14, 12, rgba(pal.ink, 0.9)); });
        fb.ttf('Source: demo loop. Hold SLAM, PUMP or NYC.', 326, 296, 11, rgba(pal.ink, 0.72), { family: 'sans', maxW: 286 });
      },
    }),
  });
})();
