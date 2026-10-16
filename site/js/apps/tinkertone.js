/* Tinkertone: a home-keyboard style instrument with 22 additive tones and six auto-accompaniment rhythms.
   The tone harmonics and rhythm grids are the app's own data (src/apps/tinkertone.rs). */
(() => {
  'use strict';
  const R = window.ROVE, A = R.audio, { ctl, fmt, rgba } = R, S = R.slint;
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const PAL = S.pal('#ECE4D0', '#2B2620', '#D2532C', '#8A7F6C');
  const D = R.data.tinkertone, TONES = D.tones, RH = D.rhythms;
  const CHORDS = ['Major', 'Minor', 'Minor 7th'], SLOTS = [0, 3, 6, 9];     // Piano, Organ, Strings, Flute
  const waves = new Map();
  const wave = (i) => {
    if (!waves.has(i)) { const h = TONES[i].h, re = new Float32Array(13), im = new Float32Array(13); h.forEach((v, k) => { im[k + 1] = v; }); waves.set(i, A.ctx.createPeriodicWave(re, im)); }
    return waves.get(i);
  };
  const midiOf = (p) => 60 + p;                                              // pads run C4.. in reading order

  R.registerApp({
    id: 'tinkertone', name: 'Tinkertone', category: 'instrument',
    desc: 'A home keyboard: 22 tones, six rhythms with auto bass and drums, and a manual bass.',
    create: R.playApp({
      name: 'Tinkertone', pal: PAL, crumb: 'HOME KEYBOARD',
      kit(inst) {
        const c = [
          ctl.f('Volume', 0, 1, 0.8, { fmt: fmt.pct, onChange: (v) => inst.ch.setLevel(v * 1.1) }), ctl.f('Accomp', 0, 1, 0.7, { fmt: fmt.pct }),
          ctl.f('Tempo', 50, 240, 84, { fmt: (v) => `${Math.round(v)} BPM` }), ctl.choice('Rhythm', RH.map((r) => r.name), 0),
          ctl.int('Tone', 0, TONES.length - 1, 0, { fmt: (v) => TONES[v].name }), ctl.choice('Chord', CHORDS, 0), ctl.bool('Vibrato', 0), ctl.bool('Sustain', 0),
        ];
        inst.c = c; inst.root = 0; inst.step = 0; inst.hits = [0, 0, 0, 0, 0]; inst.bassOn = 0; inst.sched = null;
        return { controls: c, hero: [[0, 1], [2, 3], [4, 5], [6, 7]], layers: ['KEYS'], browse: 4, routes: { sx: 2, sy: 0 } };
      },
      padLabel: (inst, p) => R.noteName(midiOf(p)),
      padState: (inst, p) => (midiOf(p) % 12 === 0 ? 1 : 0),
      line: (inst) => `${TONES[inst.c[4].value].name}  ${RH[inst.c[3].value].name} beat ${Math.floor((inst.step % (RH[inst.c[3].value].steps * 4)) / RH[inst.c[3].value].spb) % 16 + 1}`,
      enter(inst) { ensure(inst); },
      leave(inst) { },
      noteOn(inst, pad, vel, t) {
        ensure(inst); const tone = TONES[inst.c[4].value], f = A.midiHz(midiOf(pad) + tone.oct * 12 - (tone.oct > 0 ? 0 : 0));
        inst.root = midiOf(pad) % 12;
        return toneVoice(inst, tone, t, f, vel);
      },
      transport: (inst) => ({ label: () => (inst.sched && inst.sched.on ? 'STOP' : 'PLAY'), running: () => !!(inst.sched && inst.sched.on), toggle: () => toggle(inst) }),
      drawRight: draw,
    }),
  });

  function ensure(inst) {
    if (inst.out || !A.ctx) return;
    inst.out = A.gain(1); inst.out.connect(inst.ch.input);
    inst.vib = A.gain(0); const l = A.osc('sine', 5.5, A.T()); l.connect(inst.vib); l.start();
    inst.acc = A.gain(1); inst.acc.connect(inst.ch.input);
  }
  function toneVoice(inst, tone, t, f, vel) {
    const o = A.ctx.createOscillator(); o.setPeriodicWave(wave(inst.c[4].value)); o.frequency.setValueAtTime(f, t);
    if (inst.c[6].value) { const vg = A.gain(18); inst.vib.connect(vg); vg.connect(o.detune); inst.vib.gain.value = 1; }
    const env = A.gain(0), rel = A.gain(1), peak = 0.38 * vel; env.gain.setValueAtTime(0, t); env.gain.linearRampToValueAtTime(peak, t + Math.max(0.002, tone.a));
    if (tone.s === 0) A.expTo(env.gain, 0.0006, t + tone.a + Math.max(0.2, tone.d)); else env.gain.setTargetAtTime(peak * tone.s, t + tone.a, Math.max(0.01, tone.d / 3));
    o.connect(env); env.connect(rel); rel.connect(inst.out); o.start(t); o.stop(t + 30);
    return { stop(t2) { const r = tone.r + (inst.c[7].value ? 1.2 : 0); if (tone.s === 0 && !inst.c[7].value) return; A.release(rel, [o], t2, Math.max(0.05, r)); } };
  }

  function toggle(inst) {
    if (!A.ctx) return;
    if (inst.sched && inst.sched.on) { inst.sched.stop(); inst.ch.active = inst.ch.active; return; }
    inst.step = 0;
    inst.sched = new A.Scheduler(() => { const r = RH[inst.c[3].value]; return 60 / inst.c[2].value / r.spb; }, (s, t) => {
      const r = RH[inst.c[3].value], bar = Math.floor(s / r.steps) % 4, st = s % r.steps, grid = bar === 3 ? r.drums4 : r.drums, bass = (bar === 3 ? r.bass4 : r.bass)[st], vol = inst.c[1].value;
      inst.step = s;
      const names = ['kick', 'snare', 'chat', 'ohat', 'crash'];
      grid.forEach((row, i) => { if (row[st] === 'x') { A.drum(names[i], inst.acc, t, 0.55 * vol); inst.hits[i] = t; } });
      if (bass !== '.') {
        const third = inst.c[5].value === 0 ? 4 : 3, top = inst.c[5].value === 2 ? 10 : 12, off = { 0: 0, 3: third, 5: 7, 8: top, L: -5 }[bass];
        if (off != null) { const v = A.subVoice(inst.acc, t, A.midiHz(36 + inst.root + off), 0.9 * vol, { wave: 'triangle', sub: 0.3, cutoff: 700, res: 1, envAmt: 1, dec: 0.2, sus: 0.7, rel: 0.05, level: 0.55 }); v.stop(t + (60 / inst.c[2].value / r.spb) * 1.6); inst.bassOn = t; }
      }
    });
    inst.sched.start();
  }

  /* ---- the "home keyboard" card ---- */
  function draw(fb, inst, pal) {
    S.card(fb, pal, 'HOME KEYBOARD / 37 + 15 KEYS');
    const tone = TONES[inst.c[4].value], r = RH[inst.c[3].value], now = A.T();
    fb.ttf(tone.name, 326, 82, 14, pal.accent, { family: 'sans', weight: 700 });
    [['VIBRATO', 410, inst.c[6].value], ['SUSTAIN', 505, inst.c[7].value]].forEach(([n, x, on]) => { fb.disc(x, 76, 8, on ? pal.accent : rgba(pal.ink, 0.25)); fb.ttf(n, x + 14, 83, 12, rgba(pal.ink, 0.8), { spacing: 1 }); });
    // three octaves of keys, pitch classes light when held
    const held = new Set([...inst.voices.keys()].map(midiOf)), kx = 326, ky = 92, kw = 286, kh = 52, whites = [0, 2, 4, 5, 7, 9, 11], blacks = [[1, 0], [3, 1], [6, 3], [8, 4], [10, 5]];
    const lo = 60, nW = 21, w = kw / nW;
    for (let i = 0; i < nW; i++) { const m = lo - 12 + Math.floor(i / 7) * 12 + whites[i % 7]; fb.rr(kx + i * w, ky, w - 1, kh, 2, held.has(m) ? pal.accent : '#FBF8EE', rgba(pal.ink, 0.35)); }
    for (let i = 0; i < nW; i++) {
      const oct = Math.floor(i / 7), d = i % 7, m0 = lo - 12 + oct * 12;
      blacks.forEach(([pc, after]) => { if (d === after && i < nW - 1) { const m = m0 + pc; fb.rr(kx + (i + 1) * w - w * 0.3, ky, w * 0.6, kh * 0.62, 1.5, held.has(m) ? pal.accent : '#2B2620'); } });
    }
    fb.rect(kx + w * 7.2, ky + kh + 3, w * 7 * 1.0, 3, pal.accent);
    fb.ttf('BAS', 326, 168, 12, pal.ink, { weight: 700 });
    for (let i = 0; i < 16; i++) fb.rr(358 + i * 16.2, 158, 14, 12, 2, rgba(pal.ink, i === (inst.step % 16) && inst.sched && inst.sched.on ? 0.45 : 0.12));
    [0, 3, 6, 9].forEach((ti, k) => {
      const x = 326 + k * 72, on = inst.c[4].value === ti;
      fb.rr(x, 178, 68, 24, 3, on ? pal.accent : rgba(pal.ink, 0.1));
      fb.ttf(`TONE ${k + 1}`, x + 4, 189, 11, on ? pal.paper : rgba(pal.ink, 0.8)); fb.ttf(TONES[ti].name, x + 4, 200, 11, on ? pal.paper : rgba(pal.ink, 0.8), { maxW: 60 });
    });
    fb.ttf(`${r.name}  ${Math.round(inst.c[2].value)} BPM`, 326, 222, 13, pal.ink, { family: 'sans', weight: 700 });
    const run = inst.sched && inst.sched.on;
    [['RUN', 430, run], ['SYNC', 478, false], ['FILL', 540, run && Math.floor(inst.step / r.steps) % 4 === 3]].forEach(([n, x, on]) => { fb.disc(x, 214, 8, on ? pal.accent : rgba(pal.ink, 0.25)); fb.ttf(n, x + 12, 222, 12, rgba(pal.ink, 0.7), { spacing: 1 }); });
    for (let i = 0; i < 4; i++) fb.rr(326 + i * 16, 232, 13, 9, 2, rgba(pal.ink, 0.14));
    ['BD', 'SD', 'CH', 'OH', 'CY'].forEach((n, i) => { const x = 326 + i * 38; fb.disc(x, 248, 8, now - inst.hits[i] < 0.12 && run ? pal.accent : rgba(pal.ink, 0.15)); fb.ttf(n, x + 11, 256, 12, rgba(pal.ink, 0.75)); });
    fb.ttf('MANUAL BASS', 612, 256, 12, rgba(pal.ink, 0.7), { align: 'right' });
    [['VOL', inst.c[0].value, 326], ['ACC', inst.c[1].value, 410], ['OUT', Math.min(1, inst.ch.rms() * 4), 494]].forEach(([n, v, x]) => { fb.ttf(n, x, 276, 12, rgba(pal.ink, 0.8)); S.meter(fb, pal, x + 32, 270, 44, 4, v, pal.accent); });
  }
})();
