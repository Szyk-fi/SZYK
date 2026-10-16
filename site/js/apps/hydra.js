/* Hydra: an all-in-one hybrid synth with a folder-and-preset library. Layout and control names follow
   the real app; this browser version has a small library and three engines (analog, FM, plucked string). */
(() => {
  'use strict';
  const R = window.ROVE, A = R.audio, { ctl, fmt } = R, S = R.slint;
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const PAL = S.pal('#14191F', '#E7EDF4', '#A5BCE9', '#8994AA');
  const cutoffHz = (v) => 60 * (16000 / 60) ** v;
  const LAYOUT = { base: 48, layout: 'scale', scale: 'major' };
  // folder -> presets: [name, engine, params]; cut/res/atk/rel/det/drv/cho/dly/vrb are 0..1 knob values
  const LIB = [
    ['Lead', [
      ['Super Saw', 'saw', { cut: 0.75, res: 0.15, atk: 0.01, rel: 0.25, det: 0.55, drv: 0.1, cho: 0.5, dly: 0.15, vrb: 0.2, uni: 5 }],
      ['Pulse Lead', 'saw', { cut: 0.62, res: 0.3, atk: 0.005, rel: 0.15, det: 0.2, drv: 0.2, cho: 0.2, dly: 0.2, vrb: 0.15, uni: 2, wave: 'square' }],
      ['Glass Lead', 'fm', { cut: 0.9, res: 0.1, atk: 0.005, rel: 0.4, det: 0.1, drv: 0, cho: 0.3, dly: 0.25, vrb: 0.3, ratio: 3, idx: 2.5, dec: 1.0 }],
    ]],
    ['Bass', [
      ['Sub Wobble', 'saw', { cut: 0.32, res: 0.45, atk: 0.003, rel: 0.1, det: 0.1, drv: 0.45, cho: 0, dly: 0, vrb: 0.05, uni: 1, sub: 0.9 }],
      ['Acid Line', 'saw', { cut: 0.28, res: 0.7, atk: 0.002, rel: 0.08, det: 0, drv: 0.3, cho: 0, dly: 0.1, vrb: 0.05, uni: 1, sub: 0.3, env: 3 }],
      ['FM Bass', 'fm', { cut: 0.6, res: 0, atk: 0.002, rel: 0.15, det: 0, drv: 0.2, cho: 0, dly: 0, vrb: 0.05, ratio: 1, idx: 3.2, dec: 0.35, oct: -1 }],
    ]],
    ['Pad', [
      ['Warm Glass', 'saw', { cut: 0.5, res: 0.12, atk: 0.5, rel: 0.9, det: 0.4, drv: 0, cho: 0.8, dly: 0.2, vrb: 0.6, uni: 4, wave: 'triangle' }],
      ['Slow Strings', 'saw', { cut: 0.55, res: 0.05, atk: 0.8, rel: 1.2, det: 0.5, drv: 0, cho: 0.9, dly: 0, vrb: 0.5, uni: 5 }],
    ]],
    ['Pluck', [
      ['Nylon', 'pluck', { cut: 0.8, res: 0, atk: 0, rel: 0.2, det: 0, drv: 0, cho: 0.2, dly: 0.2, vrb: 0.3, damp: 0.3, t60: 2.2 }],
      ['Harp', 'pluck', { cut: 0.85, res: 0, atk: 0, rel: 0.2, det: 0, drv: 0, cho: 0.3, dly: 0.3, vrb: 0.5, damp: 0.15, t60: 3.4 }],
      ['Marimba', 'fm', { cut: 0.8, res: 0, atk: 0.002, rel: 0.2, det: 0, drv: 0, cho: 0, dly: 0.1, vrb: 0.25, ratio: 4, idx: 1.2, dec: 0.6 }],
    ]],
    ['Keys', [
      ['E Piano', 'fm', { cut: 0.8, res: 0, atk: 0.003, rel: 0.4, det: 0.1, drv: 0, cho: 0.5, dly: 0.1, vrb: 0.25, ratio: 1, idx: 1.6, dec: 1.6 }],
      ['Bell Keys', 'fm', { cut: 0.95, res: 0, atk: 0.003, rel: 0.5, det: 0, drv: 0, cho: 0.2, dly: 0.25, vrb: 0.4, ratio: 3.5, idx: 2.0, dec: 2.5 }],
    ]],
  ];

  R.registerApp({
    id: 'hydra', name: 'Hydra', category: 'instrument',
    desc: 'An all-in-one hybrid synth: analog, FM and plucked-string engines, a folder-and-preset library, macros and effects.',
    create: R.playApp({
      name: 'Hydra', pal: PAL, crumb: 'HYBRID SYNTH',
      kit(inst) {
        const load = () => {
          const pr = LIB[inst.folder][1][inst.c[2].base % LIB[inst.folder][1].length][2], c = inst.c;
          [['cut', 0], ['res', 1], ['atk', 4], ['rel', 5], ['det', 6], ['drv', 7], ['cho', 8], ['dly', 9], ['vrb', 10]].forEach(([k, i]) => { c[i].base = clamp(pr[k] ?? 0, 0, 1); });
          update(inst);
        };
        const k = (l, def, fm) => ctl.f(l, 0, 1, def, { fmt: fm || fmt.pct, onChange: () => update(inst) });
        inst.folder = 0;
        const c = [
          k('Cutoff', 0.75, (v) => fmt.hz(cutoffHz(v))), k('Resonance', 0.15),
          ctl.int('Preset', 0, 2, 0, { fmt: () => LIB[inst.folder][1][inst.c[2].base % LIB[inst.folder][1].length][0], onChange: () => load() }),
          ctl.int('Folder', 0, LIB.length - 1, 0, { fmt: (v) => LIB[v][0], onChange: (v) => { inst.folder = v; inst.c[2].base = 0; load(); } }),
          k('Attack', 0.1), k('Release', 0.3), k('Detune', 0.4), k('Drive', 0.1), k('Chorus', 0.5), k('Delay', 0.15), k('Reverb', 0.2),
          ctl.f('Level', 0, 1, 0.8, { fmt: fmt.pct, onChange: (v) => inst.ch.setLevel(v * 1.1) }), ctl.int('Octave', -2, 2, 0, { fmt: (v) => `${v >= 0 ? '+' : ''}${v}` }),
        ];
        inst.c = c; setTimeout(load, 0);
        return { controls: c, hero: [[0, 1], [2, 3], [4, 5], [6, 7], [8, 9], [10, 11]], layers: ['KEYS'], browse: 2, routes: { sx: 0, sy: 1, hl: 8, hr: 10 } };
      },
      padLabel: (inst, p) => R.noteName(R.padNote(p, { ...LAYOUT, transpose: inst.c[12].value * 12 })),
      padState: (inst, p) => (R.padNote(p, LAYOUT) % 12 === 0 ? 1 : 0),
      line: () => 'Play',
      enter(inst) { ensure(inst); },
      noteOn(inst, pad, vel, t) {
        ensure(inst); const [name, eng, pr] = LIB[inst.folder][1][inst.c[2].base % LIB[inst.folder][1].length], c = inst.c;
        const midi = R.padNote(pad, { ...LAYOUT, transpose: c[12].value * 12 }) + (pr.oct || 0) * 12, f = A.midiHz(midi), cut = cutoffHz(c[0].value), q = 0.5 + c[1].value * 12;
        if (inst.voices.size >= 10) { const k = inst.voices.keys().next().value; inst.voices.get(k).stop(A.T()); inst.voices.delete(k); }
        let v;
        if (eng === 'saw') v = A.subVoice(inst.fx.in, t, f, vel, { wave: pr.wave || 'sawtooth', mix2: Math.min(1, (pr.uni || 2) / 3), detune: c[6].value * 40, sub: pr.sub || 0, cutoff: cut, res: q, envAmt: pr.env || 0.8, atk: 0.002 + c[4].value ** 2 * 1.5, dec: 0.4, sus: 0.8, rel: 0.05 + c[5].value ** 2 * 2.5, level: 0.4 });
        else if (eng === 'fm') v = A.fmVoice(inst.fx.in, t, f, vel, { rc: 1, r1: pr.ratio, r2: pr.ratio * 2, i1: pr.idx, i2: pr.idx * 0.3, decay: pr.dec * (0.5 + c[5].value), atk: 0.002, level: 0.5 });
        else v = A.pluckVoice(inst.fx.in, t, f, vel, { damp: pr.damp, t60: pr.t60 * (0.5 + c[5].value), level: 0.9 });
        return v;
      },
      drawRight(fb, inst, pal) { S.scalePanel(fb, pal, { root: 'C', name: 'Major / Ionian', notes: 'C · D · E · F · G · A · B', lit: [1, 0, 1, 0, 1, 1, 0, 1, 0, 1, 0, 1] }); },
    }),
  });
  function ensure(inst) { if (!inst.fx && A.ctx) { inst.fx = A.insertFx(inst.ch); update(inst); } }
  function update(inst) { if (!inst.fx) return; const c = inst.c; inst.fx.set({ chorus: c[8].value, delay: c[9].value * 0.6, dtime: 0.34, fb: 0.35, verb: c[10].value }); }
})();
