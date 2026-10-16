/* Blaster: hold a pad to charge, let go to fire. Controls and character names follow the real
   app; the voice is a Web Audio version (a climbing, chirping charge, then a swept blast). */
(() => {
  'use strict';
  const R = window.ROVE, A = R.audio, { ctl, fmt } = R, S = R.slint;
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const PAL = S.pal('#14191F', '#E7EDF4', '#A5BCE9', '#8994AA');
  const WAVES = ['Reed', 'Saw', 'Square', 'Sine'], OSC = ['sawtooth', 'sawtooth', 'square', 'sine'];
  // charge time s, blast ms, charge pitch octave offset, climb oct, sub charge, chirp depth, blast start x, blast end x, sweep ms, noise, body, sub blast, echo
  const CHARS = [
    ['X Charge Shot', 1.4, 100, 0, 1.0, 0.3, 0.5, 4, 0.6, 180, 0.72, 0, 0.2, 0.3],
    ['Buster Charge', 1.0, 160, 0, 0.8, 0.2, 0.6, 3, 0.5, 220, 0.4, 2, 0.4, 0.2],
    ['Pea Shot', 0.5, 70, 1, 0.3, 0.0, 0.2, 5, 1.0, 90, 0.2, 2, 0.0, 0.0],
    ['Plasma Beam', 1.8, 500, 0, 1.5, 0.5, 0.8, 2.5, 0.4, 380, 0.5, 1, 0.3, 0.5],
    ['Fireball', 1.2, 320, -1, 0.9, 0.4, 0.3, 2, 0.3, 260, 0.9, 1, 0.5, 0.2],
    ['Laser Rifle', 0.6, 180, 1, 1.2, 0.1, 0.9, 6, 0.3, 140, 0.15, 2, 0.1, 0.3],
    ['Thunder Strike', 1.6, 420, -1, 0.6, 0.6, 0.1, 3, 0.5, 300, 1.0, 1, 0.7, 0.4],
    ['Wave Cannon', 2.2, 800, -1, 1.8, 0.7, 0.9, 1.8, 0.3, 600, 0.6, 3, 0.8, 0.6],
    ['Magic Orb', 1.1, 260, 1, 0.7, 0.8, 0.2, 2.2, 0.6, 200, 0.2, 3, 0.1, 0.7],
    ['Power Fist', 0.8, 130, -2, 0.5, 0.1, 0.5, 2.5, 0.2, 110, 0.8, 1, 0.9, 0.0],
  ];
  const KEYS = ['chargeTime', 'length', null, 'level', 'chgPitch', 'climb', 'subCharge', 'chirp', 'blastStart', 'blastEnd', 'sweep', 'noise', 'body', 'subBlast', 'echo', 'auto'];

  R.registerApp({
    id: 'blaster', name: 'Blaster', category: 'instrument',
    desc: 'Hold a key to charge up, let go to fire. Design charge shots, beams and blasts like video-game weapons.',
    create: R.playApp({
      name: 'Blaster', pal: PAL, crumb: 'CHARGE SHOTS',
      kit(inst) {
        const set = (i) => { const c = CHARS[i]; const v = { chargeTime: c[1], length: c[2], chgPitch: c[3], climb: c[4], subCharge: c[5], chirp: c[6], blastStart: c[7], blastEnd: c[8], sweep: c[9], noise: c[10], body: c[11], subBlast: c[12], echo: c[13] }; Object.keys(v).forEach((k) => { const ctrl = inst.byKey[k]; if (ctrl) ctrl.base = v[k]; }); };
        const f = (l, a, b, d, fm, o = {}) => ctl.f(l, a, b, d, { fmt: fm, ...o });
        const c = [
          f('Charge time', 0.3, 2.5, 1.4, (v) => `${v.toFixed(2)} s`, { log: true }), f('Blast length', 40, 1200, 100, (v) => `${Math.round(v)} ms`, { log: true }),
          ctl.int('Character', 0, CHARS.length - 1, 0, { fmt: (v) => CHARS[v][0], onChange: set }),
          f('Level', 0, 1, 0.7, fmt.pct, { onChange: (v) => inst.ch.setLevel(v * 1.2) }),
          f('Charge pitch', -2, 2, 0, (v) => fmt.st(v * 12)), f('Climb', 0, 2, 1, (v) => `${v.toFixed(2)} oct`), f('Sub (charge)', 0, 1, 0.3, fmt.pct), f('Chirp depth', 0, 1, 0.5, fmt.pct),
          f('Blast start', 1, 8, 4, (v) => `${v.toFixed(1)}×`), f('Blast end', 0.2, 2, 0.6, (v) => `${v.toFixed(1)}×`), f('Sweep time', 40, 800, 180, (v) => `${Math.round(v)} ms`, { log: true }),
          f('Noise', 0, 1, 0.72, fmt.pct), ctl.choice('Body', WAVES, 0), f('Sub (blast)', 0, 1, 0.2, fmt.pct), f('Echo', 0, 1, 0.3, fmt.pct), ctl.bool('Fire at full', 0),
        ];
        inst.byKey = {}; KEYS.forEach((k, i) => { if (k) inst.byKey[k] = c[i]; });
        inst.c = c; inst.charge = 0; inst.flash = 0;
        return { controls: c, hero: [[0, 1], [2, 3], [4, 5], [6, 7], [8, 9], [10, 11], [12, 13], [14, 15]], layers: ['SHOOT'], browse: 2, routes: { sx: 0, sy: 1, hl: 5, hr: 10 } };
      },
      padLabel: (inst, p) => R.noteName(R.padNote(p, { base: 48, layout: 'scale', scale: 'major' })),
      padState: (inst, p) => ((R.padNote(p, { base: 48, layout: 'scale', scale: 'major' }) % 12) === 0 ? 1 : 0),
      line: (inst) => `X ${CHARS[inst.c[2].value][0].replace(/^X /, '')}  charge ${Math.round(inst.charge * 100)}%`,
      enter(inst) { ensure(inst); },
      noteOn(inst, pad, vel, t) { ensure(inst); return charge(inst, t, A.midiHz(R.padNote(pad, { base: 48, layout: 'scale', scale: 'major' })), vel); },
      tick(inst) { inst.charge = inst.active ? clamp((A.T() - inst.active.t) / inst.active.ct, 0, 1) : inst.charge * 0.9; if (inst.flash > 0) inst.flash--; },
      drawRight(fb, inst, pal) { S.scalePanel(fb, pal, { root: 'C', name: 'Major / Ionian', notes: 'C · D · E · F · G · A · B', lit: [1, 0, 1, 0, 1, 1, 0, 1, 0, 1, 0, 1] }); },
    }),
  });

  function ensure(inst) {
    if (inst.out || !A.ctx) return;
    inst.out = A.gain(1);
    const dl = A.ctx.createDelay(1), fb = A.gain(0.35), mix = A.gain(0); inst.out.connect(dl); dl.delayTime.value = 0.16; dl.connect(fb); fb.connect(dl); dl.connect(mix); mix.connect(inst.ch.input);
    inst.echo = mix; inst.out.connect(inst.ch.input);
    inst.ch.setLevel(inst.c[3].base * 1.2);
  }
  function charge(inst, t, f0, vel) {
    const p = (k) => inst.byKey[k].value, ct = p('chargeTime'), f = f0 * 2 ** p('chgPitch');
    inst.echo.gain.setTargetAtTime(p('echo') * 0.6, A.T(), 0.05);
    const body = A.osc('square', f * 0.5, t); body.frequency.linearRampToValueAtTime(f * 0.5 * 2 ** p('climb'), t + ct);
    const lfo = A.osc('sawtooth', 2.5, t); A.expTo(lfo.frequency, 20, t + ct); const lg = A.gain(500 * p('chirp')); lfo.connect(lg); lg.connect(body.detune);
    const lp = A.filt('lowpass', 1400, 1.2); lp.frequency.linearRampToValueAtTime(5200, t + ct);
    const env = A.gain(0); env.gain.setValueAtTime(0, t); env.gain.linearRampToValueAtTime(0.2 * vel, t + 0.02);
    const sub = A.osc('sine', f * 0.25, t), sg = A.gain(p('subCharge') * 0.4); sub.connect(sg); sg.connect(env);
    const fl = A.osc('sine', 27, t), fg = A.gain(0); fg.gain.setValueAtTime(0, t + ct); fg.gain.linearRampToValueAtTime(0.1 * vel, t + ct + 0.05); fl.connect(fg); fg.connect(env.gain);
    const rel = A.gain(1); body.connect(lp); lp.connect(env); env.connect(rel); rel.connect(inst.out);
    const srcs = [body, lfo, fl, sub]; srcs.forEach((o) => { o.start(t); o.stop(t + 30); });
    inst.active = { t, ct };
    return {
      stop(t2) {
        const c = inst.byKey.auto.value ? 1 : clamp((t2 - t) / ct, 0.1, 1); inst.active = null; inst.flash = 12;
        A.release(rel, srcs, t2, 0.03); fire(inst, t2, c, f, vel);
      },
    };
  }
  function fire(inst, t, c, f, vel) {
    const p = (k) => inst.byKey[k].value, len = (p('length') / 1000) * (0.55 + 0.45 * c) * 2.2, amp = 0.5 * (0.35 + 0.65 * c) * vel, sw = p('sweep') / 1000;
    const o = A.osc(OSC[p('body')], f * p('blastStart') * (0.5 + 0.5 * c), t); A.expTo(o.frequency, Math.max(30, f * p('blastEnd')), t + Math.max(0.03, sw));
    const sub = A.osc('sine', f * 0.5 * (1 + c), t); A.expTo(sub.frequency, 32, t + len * 0.8);
    const lp = A.filt('lowpass', 7000, 0.8); lp.frequency.setValueAtTime(7000, t); A.expTo(lp.frequency, 500, t + len);
    const g = A.gain(0); g.gain.setValueAtTime(amp, t); A.expTo(g.gain, 0.0003, t + len);
    const sg = A.gain(0); sg.gain.setValueAtTime(amp * 1.4 * p('subBlast'), t); A.expTo(sg.gain, 0.0003, t + len * 0.9);
    o.connect(lp); lp.connect(g); g.connect(inst.out); sub.connect(sg); sg.connect(inst.out);
    o.start(t); sub.start(t); o.stop(t + len + 0.05); sub.stop(t + len + 0.05);
    A.dNoise(inst.out, 'bandpass', 5000, 0.9, t, (0.1 + 0.5 * c) * p('noise'), len * 0.5, 350);
  }
})();
