/* Vistas: generative cinematic scapes. Pick a mood and a key and it composes forever: a voice-led chord
   progression, bass, a shimmer layer and ambience. The moods are real modes, as in the app. */
(() => {
  'use strict';
  const R = window.ROVE, A = R.audio, { ctl, fmt, rgba } = R, S = R.slint;
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const PAL = S.pal('#14191F', '#E7EDF4', '#A5BCE9', '#8994AA');
  const KEYS = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
  // mood -> [short pad label, long name, mode intervals, chord degrees pool]
  const MOODS = [
    ['HOPE', 'Hopeful', [0, 2, 4, 5, 7, 9, 11], [0, 4, 5, 3, 1]], ['DREM', 'Dreamy', [0, 2, 4, 6, 7, 9, 11], [0, 1, 4, 5]], ['PEAC', 'Peaceful', [0, 2, 4, 7, 9, 11, 12], [0, 3, 4, 0]],
    ['NOST', 'Nostalgic', [0, 2, 4, 5, 7, 9, 10], [0, 5, 3, 4]], ['MELA', 'Melancholy', [0, 2, 3, 5, 7, 8, 10], [0, 5, 2, 6]], ['MYST', 'Mysterious', [0, 2, 3, 5, 7, 9, 10], [0, 1, 3, 6]],
    ['EPIC', 'Epic', [0, 2, 3, 5, 7, 8, 10], [0, 5, 6, 4]], ['TENS', 'Tense', [0, 1, 3, 5, 6, 8, 10], [0, 1, 5, 4]], ['DARK', 'Dark', [0, 1, 3, 5, 7, 8, 10], [0, 1, 0, 5]], ['OMIN', 'Open minor', [0, 2, 3, 5, 7, 8, 10], [0, 2, 5, 3]],
  ];
  const MODES = ['Loop', 'Compose', 'Evolve'];

  R.registerApp({
    id: 'vistas', name: 'Vistas', category: 'instrument',
    desc: 'Generative cinematic scapes: choose a mood and a key and it composes chord progressions, bass and shimmer forever.',
    create: R.playApp({
      name: 'Vistas', pal: PAL, crumb: 'CINEMATIC SCAPES',
      kit(inst) {
        const c = [
          ctl.int('Mood', 0, MOODS.length - 1, 0, { fmt: (v) => MOODS[v][1] }), ctl.choice('Key', KEYS, 7),
          ctl.f('Variation', 0, 1, 0.45, { fmt: fmt.pct }), ctl.int('Complexity', 1, 8, 4, { fmt: (v) => `${v} notes` }),
          ctl.f('Duration', 3, 16, 6, { fmt: (v) => `${v.toFixed(1)} s` }), ctl.f('Reverb', 0, 1, 0.6, { fmt: fmt.pct }),
          ctl.choice('Mode', MODES, 0),
        ];
        inst.c = c; inst.run = false; inst.chord = 0; inst.chordNotes = []; inst.progress = 0; inst.bar = 0; inst.kitFlash = 0;
        return { controls: c, hero: [[0, 1], [2, 3], [4, 5]], layers: ['MOOD'], browse: 0, routes: { sx: 5, sy: 2 } };
      },
      // the pad map: moods on the bottom two rows and a half, then DARK / OMIN, PLAY / NEW, and the mode pads on top
      padLabel: (inst, p) => { const r = R.rank(p); return r < 10 ? MOODS[r][0] : ['PLAY', 'NEW', 'LOOP', 'COMP', 'EVOL', 'NEXT'][r - 10]; },
      padState: (inst, p) => { const r = R.rank(p); return r < 10 ? (inst.c[0].value === r ? 1 : 0) : r === 10 ? (inst.run ? 3 : 0) : r >= 12 && r <= 14 ? (inst.c[6].value === r - 12 ? 1 : 0) : 0; },
      line: (inst) => (inst.run ? `${MODES[inst.c[6].value]} · ${KEYS[inst.c[1].value]} ${MOODS[inst.c[0].value][1].toLowerCase()}` : 'Play'),
      noteOn(inst, p) {
        const r = R.rank(p);
        if (r < 10) { inst.c[0].base = r; inst.kit.flash(MOODS[r][1]); } else if (r === 10) toggle(inst); else if (r === 11) { inst.chord = 0; inst.chordNotes = []; inst.nextAt = 0; inst.queued = 0; inst.kit.flash('New scape'); } else if (r <= 14) inst.c[6].base = r - 12; else { inst.nextAt = 0; inst.queued = 0; inst.kit.flash('Next'); }
        return null;
      },
      transport: (inst) => ({ label: () => (inst.run ? 'STOP' : 'PLAY'), running: () => inst.run, toggle: () => toggle(inst) }),
      enter(inst) { ensure(inst); },
      tick(inst, inp) {
        if (!inst.run || !A.ctx) return;
        const now = A.T();
        if (!inst.nextAt || now >= inst.nextAt - 0.5) { if (!inst.queued || now >= inst.queued) { play(inst, Math.max(now + 0.05, inst.nextAt || 0)); } }
        inst.progress = inst.nextAt ? clamp(1 - (inst.nextAt - now) / inst.dur, 0, 1) : 0;
      },
      drawRight(fb, inst, pal) {
        const m = MOODS[inst.c[0].value];
        S.card(fb, pal, 'VOICE / CONTROL');
        fb.ttf('Mood', 324, 100, 24, pal.ink, { family: 'sans', weight: 700 }); fb.rect(324, 110, 290, 2, pal.accent);
        fb.ttf(m[1], 324, 138, 18, pal.accent, { family: 'sans' });
        // a field of stars and the four layers' levels, as on the real screen
        for (let i = 0; i < 46; i++) { const x = 330 + ((i * 97) % 280), y = 136 + ((i * 53) % 100), tw = 0.3 + 0.7 * Math.abs(Math.sin(inst.frame * 0.03 + i)); fb.rect(x, y, 1.5, 1.5, rgba(pal.ink, tw * 0.6)); }
        const chordName = inst.chordNotes.length ? R.noteName(inst.chordNotes[0]).replace(/\d/g, '') : '--';
        fb.ttf(chordName, 324, 200, 28, rgba(pal.ink, 0.9), { family: 'sans', weight: 600 });
        fb.rr(324, 214, 290, 6, 3, rgba(pal.ink, 0.13)); fb.rr(324, 214, Math.max(6, 290 * inst.progress), 6, 3, pal.accent);
        ['VAR', 'CPX', 'DUR', 'REV'].forEach((n, i) => { const v = [inst.c[2].text, inst.c[3].text, inst.c[4].text, inst.c[5].text][i]; fb.ttf(n, 324 + (i % 2) * 150, 244 + Math.floor(i / 2) * 16, 12, rgba(pal.ink, 0.6), { spacing: 1 }); fb.ttf(v, 366 + (i % 2) * 150, 244 + Math.floor(i / 2) * 16, 12, pal.ink); });
        fb.ttf('Up/down picks a row; left/right changes it.', 324, 296, 12, rgba(pal.ink, 0.72), { family: 'sans' });
      },
    }),
  });

  function ensure(inst) {
    if (inst.fx || !A.ctx) return;
    inst.fx = A.insertFx(inst.ch); inst.fx.set({ chorus: 0.5, delay: 0.2, dtime: 0.45, fb: 0.4, verb: inst.c[5].value });
    inst.bus = A.gain(1); inst.bus.connect(inst.fx.in);
  }
  function toggle(inst) { if (!A.ctx) return; ensure(inst); inst.run = !inst.run; inst.nextAt = 0; inst.ch.active = inst.run; if (inst.run) { inst.chord = 0; } }
  function chordFor(inst) {
    const mood = MOODS[inst.c[0].value], key = inst.c[1].value, scale = mood[2], pool = mood[3], n = inst.c[3].value, mode = inst.c[6].value;
    let deg = pool[inst.chord % pool.length];
    if (mode !== 0 && Math.random() < inst.c[2].value) deg = pool[Math.floor(Math.random() * pool.length)];
    const root = 48 + key + scale[deg % 7], notes = [];
    for (let k = 0; k < n; k++) { const d = deg + k * 2; notes.push(48 + key + scale[d % 7] + 12 * Math.floor(d / 7) + (k > 3 ? 12 : 0)); }
    return { root, notes };
  }
  function play(inst, t) {
    const { root, notes } = chordFor(inst), dur = inst.c[4].value, c = A.ctx;
    inst.dur = dur; inst.chord++; inst.chordNotes = notes; inst.nextAt = t + dur; inst.queued = t + dur - 0.4;
    inst.fx.set({ verb: inst.c[5].value });
    notes.forEach((m, i) => {
      const v = A.subVoice(inst.bus, t + i * 0.03, A.midiHz(m), 0.55, { wave: 'sawtooth', mix2: 0.8, detune: 14, cutoff: 900 + inst.c[2].value * 1500, res: 0.7, atk: dur * 0.35, dec: dur * 0.3, sus: 0.8, rel: dur * 0.5, level: 0.16 });
      v.stop(t + dur * 0.95);
    });
    const b = A.subVoice(inst.bus, t, A.midiHz(root - 12), 0.8, { wave: 'sine', sub: 0.4, cutoff: 260, atk: 0.8, dec: 1, sus: 0.9, rel: 1.5, level: 0.5 }); b.stop(t + dur * 0.97);
    // shimmer: soft bell notes high in the chord, more of them with more variation
    const bells = 2 + Math.round(inst.c[2].value * 6);
    for (let i = 0; i < bells; i++) A.fmVoice(inst.bus, t + Math.random() * dur * 0.9, A.midiHz(notes[i % notes.length] + 24), 0.35, { rc: 1, r1: 2, r2: 5, i1: 0.6, i2: 0.3, decay: 3, atk: 0.01, level: 0.28 });
    // ambience: a slow filtered noise swell
    const n = A.noiseSrc(), bp = A.filt('bandpass', 700 + Math.random() * 1500, 0.8), g = A.gain(0);
    g.gain.setValueAtTime(0, t); g.gain.linearRampToValueAtTime(0.05, t + dur * 0.5); g.gain.linearRampToValueAtTime(0, t + dur); n.connect(bp); bp.connect(g); g.connect(inst.bus); n.start(t, Math.random()); n.stop(t + dur + 0.1);
  }
})();
