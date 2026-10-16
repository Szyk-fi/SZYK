/* June-60: a polyphonic analog-style synth (DCO saw + pulse + sub + noise, resonant low-pass with its own
   envelope, a high-pass, and a chorus). The shell shows the same Voice / Control card the real app does. */
(() => {
  'use strict';
  const R = window.ROVE, A = R.audio, { ctl, fmt } = R, S = R.slint;
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const PAL = S.pal('#14191F', '#E7EDF4', '#A5BCE9', '#8994AA');
  const PATCH = [
    ['Init Saw', { cut: 0.7, res: 0, env: 0, atk: 0.01, dec: 0.3, sus: 0.8, rel: 0.2, pulse: 0, sub: 0.3, cho: 1 }],
    ['Brass', { cut: 0.45, res: 0.1, env: 0.55, atk: 0.06, dec: 0.4, sus: 0.7, rel: 0.2, pulse: 0.3, sub: 0.2, cho: 1 }],
    ['Strings', { cut: 0.6, res: 0, env: 0.05, atk: 0.5, dec: 0.5, sus: 0.9, rel: 0.7, pulse: 0.6, sub: 0, cho: 2 }],
    ['Poly Bass', { cut: 0.3, res: 0.35, env: 0.5, atk: 0.003, dec: 0.25, sus: 0.3, rel: 0.1, pulse: 0.2, sub: 0.8, cho: 0 }],
    ['Soft Pad', { cut: 0.5, res: 0.1, env: 0.15, atk: 0.9, dec: 0.6, sus: 0.9, rel: 1.2, pulse: 0.5, sub: 0.1, cho: 2 }],
    ['Pluck Lead', { cut: 0.55, res: 0.4, env: 0.7, atk: 0.002, dec: 0.18, sus: 0.1, rel: 0.15, pulse: 0.1, sub: 0.2, cho: 1 }],
    ['Organ', { cut: 0.85, res: 0, env: 0, atk: 0.005, dec: 0.1, sus: 1, rel: 0.08, pulse: 1, sub: 0.7, cho: 2 }],
    ['Sweeper', { cut: 0.25, res: 0.6, env: 0.8, atk: 0.4, dec: 1.2, sus: 0.5, rel: 0.6, pulse: 0.4, sub: 0.3, cho: 1 }],
  ];
  const hz = (v) => 60 * (14000 / 60) ** v;

  R.registerApp({
    id: 'june60', name: 'June-60', category: 'instrument',
    desc: 'A polyphonic analog-style synth: DCO, sub, noise, filter and envelope, with a stereo chorus.',
    create: R.playApp({
      name: 'June-60', title: 'June-60', pal: PAL, crumb: 'POLYSYNTH',
      kit(inst) {
        const load = (i) => { const p = PATCH[i][1], c = inst.c; c[0].base = p.cut; c[1].base = p.res; c[2].base = p.env; c[4].base = p.atk; c[5].base = p.dec; c[6].base = p.sus; c[7].base = p.rel; c[8].base = p.pulse; c[9].base = p.sub; c[10].base = p.cho; chorus(inst); };
        const f = (l, d, fm, o = {}) => ctl.f(l, 0, 1, d, { fmt: fm || fmt.pct, ...o });
        const sec = (v) => fmt.ms((v ** 2) * 3000);
        const c = [
          f('Cutoff', 0.7), f('Resonance', 0), f('Env Amount', 0), f('LFO Filter', 0), f('Attack', 0.01, sec), f('Decay', 0.3, sec), f('Sustain', 0.8), f('Release', 0.2, sec),
          f('Pulse', 0), f('Sub', 0.3), ctl.int('Chorus', 0, 2, 1, { fmt: (v) => ['off', 'I', 'II'][v], onChange: () => chorus(inst) }),
          ctl.int('Patch', 0, PATCH.length - 1, 0, { fmt: (v) => `${String(v + 1).padStart(2, '0')} ${PATCH[v][0]}`, onChange: load }),
          ctl.int('Octave', -2, 2, 0, { fmt: (v) => `${v >= 0 ? '+' : ''}${v}` }),
        ];
        inst.c = c; inst.patchIdx = 0;
        return { controls: c, hero: [[0, 1], [2, 3], [4, 5], [6, 7], [8, 9], [10, 11]], layers: ['KEYS'], browse: 11, routes: { sx: 0, sy: 3, hl: 1, hr: 2 } };
      },
      padLabel: (inst, p) => R.noteName(R.padNote(p, { base: 48 + inst.c[12].value * 12 })),
      padState: (inst, p) => (R.padNote(p, { base: 48 }) % 12 === 0 ? 1 : 0),
      line: () => 'Play',
      enter(inst) { ensure(inst); },
      noteOn(inst, pad, vel, t) {
        ensure(inst); const c = inst.c, f = A.midiHz(R.padNote(pad, { base: 48 + c[12].value * 12 }));
        if (inst.voices.size >= 6) { const k = inst.voices.keys().next().value; inst.voices.get(k).stop(A.T()); inst.voices.delete(k); }
        return A.subVoice(inst.fx.in, t, f, vel, { wave: 'sawtooth', wave2: 'square', mix2: c[8].value * 1.2, detune: 6, sub: c[9].value, noise: 0.04, cutoff: hz(c[0].value), res: 0.5 + c[1].value * 14, envAmt: c[2].value * 4, keyTrack: 0.4, atk: 0.003 + c[4].value ** 2 * 3, dec: 0.05 + c[5].value ** 2 * 3, sus: c[6].value, rel: 0.04 + c[7].value ** 2 * 3, level: 0.45 });
      },
      drawRight(fb, inst, pal) { S.paramCard(fb, pal, 'VOICE / CONTROL', 'Patch', `${String(inst.c[11].base + 1).padStart(2, '0')} ${PATCH[inst.c[11].base][0]}`); },
    }),
  });
  function ensure(inst) { if (!inst.fx && A.ctx) { inst.fx = A.insertFx(inst.ch); chorus(inst); } }
  function chorus(inst) { if (inst.fx) inst.fx.set({ chorus: [0, 0.5, 0.9][inst.c[10].value], verb: 0.05 }); }
})();
