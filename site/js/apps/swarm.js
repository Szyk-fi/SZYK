/* Swarm: a network of eight coupled oscillators that sing to each other. Each node has a phase and a
   pitch; coupled phases pull together (Kuramoto-style), and every time a node's phase wraps it plays its
   note. Cohesion is the coupling, Pitch spread widens the notes, Mutation lets pitches wander. */
(() => {
  'use strict';
  const R = window.ROVE, A = R.audio, { ctl, fmt, rgba } = R, S = R.slint;
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const PAL = S.pal('#172019', '#EDF2D9', '#D2EB8B', '#EEA777');
  const LAYOUT = { base: 48, layout: 'scale', scale: 'major' }, SC = R.SCALES.major, N = 8;

  R.registerApp({
    id: 'swarm', name: 'Swarm', category: 'instrument',
    desc: 'A network of eight coupled oscillators. They pull each other into step, and every pulse is a note.',
    create: R.playApp({
      name: 'Swarm', pal: PAL, crumb: 'GENERATIVE',
      kit(inst) {
        const c = [
          ctl.f('Cohesion', 0, 1, 0.7, { fmt: fmt.d2 }), ctl.f('Pitch spread', 0, 1, 0.2, { fmt: fmt.d2 }), ctl.f('Tone color', 0, 1, 0.5, { fmt: fmt.pct }), ctl.f('Mutation', 0, 1, 0.2, { fmt: fmt.d2 }),
          ctl.int('Scene', 0, 3, 0, { fmt: (v) => ['Drift', 'Cluster', 'Pulse', 'Rain'][v] }), ctl.f('Density', 0.2, 2, 1, { fmt: (v) => `${v.toFixed(2)}×` }),
        ];
        inst.c = c; inst.run = false; inst.step = 0; inst.pk = 0;
        inst.nodes = Array.from({ length: N }, (_, i) => ({ ph: Math.random(), w: 0.35 + Math.random() * 0.5, deg: [0, 4, 2, 7, 5, 9, 3, 6][i], flash: 0, a: (i / N) * Math.PI * 2 }));
        return { controls: c, hero: [[0, 1], [2, 3], [4, 5]], layers: ['KEYS'], browse: 4, routes: { sx: 0, sy: 1, hl: 3, hr: 2 } };
      },
      padLabel: (inst, p) => R.noteName(R.padNote(p, LAYOUT)),
      padState: (inst, p) => (R.padNote(p, LAYOUT) % 12 === 0 ? 1 : 0),
      line: (inst) => (inst.run ? `running · step ${inst.step}` : 'Pads play'),
      enter(inst) { ensure(inst); },
      noteOn(inst, pad, vel, t) { ensure(inst); const n = inst.nodes[pad % N]; n.flash = 1; n.deg = R.rank(pad); return note(inst, t, R.padNote(pad, LAYOUT), vel); },
      transport: (inst) => ({ label: () => (inst.run ? 'STOP' : 'START'), running: () => inst.run, toggle: () => { inst.run = !inst.run; inst.ch.active = inst.run; } }),
      tick(inst, inp) {
        const dt = inp.dt, c = inst.c, k = c[0].value * 1.6, dens = c[5].value;
        if (inst.run && A.ctx) {
          const sc = c[4].value;
          inst.nodes.forEach((n, i) => {
            let pull = 0; inst.nodes.forEach((m, j) => { if (i !== j) pull += Math.sin((m.ph - n.ph) * Math.PI * 2); });
            n.ph += (n.w * dens * (sc === 2 ? 1.6 : sc === 3 ? 0.7 : 1) + (k / N) * pull / (Math.PI * 2) * 3) * dt;
            if (n.ph >= 1) {
              n.ph -= Math.floor(n.ph); n.flash = 1; inst.step++;
              if (Math.random() < c[3].value * 0.35) n.deg = clamp(n.deg + (Math.random() < 0.5 ? -1 : 1) * (1 + (Math.random() * 3 | 0)), 0, 14);
              const oct = Math.floor(c[1].value * 3 * Math.random());
              note(inst, A.T() + 0.01, 48 + SC[n.deg % 7] + 12 * Math.floor(n.deg / 7) + 12 * oct, 0.6 + Math.random() * 0.3);
              inst.pk = Math.max(inst.pk, 0.12);
            }
          });
        }
        inst.nodes.forEach((n) => { n.flash *= 0.93; }); inst.pk *= 0.96;
      },
      drawRight(fb, inst, pal) {
        S.card(fb, pal, 'VOICE NETWORK / 8 NODES');
        fb.ttf(inst.run ? '● RUNNING / LIVE ENGINE' : 'STOPPED · F3 STARTS IT', 612, 94, 11, rgba(pal.ink, 0.75), { align: 'right', spacing: 1 });
        const cx = 468, cy = 164, rx = 116, ry = 52, pos = inst.nodes.map((n) => [cx + Math.cos(n.a) * rx, cy + Math.sin(n.a) * ry]);
        fb.rr(336, 106, 264, 116, 58, null, rgba(pal.ink, 0.12));
        for (let i = 0; i < N; i++) for (let j = i + 1; j < N; j++) { const al = inst.c[0].value * (0.1 + 0.5 * Math.cos((inst.nodes[i].ph - inst.nodes[j].ph) * Math.PI) ** 2); fb.g.strokeStyle = rgba(pal.accent, al * 0.6); fb.g.lineWidth = 1; fb.g.beginPath(); fb.g.moveTo(pos[i][0], pos[i][1]); fb.g.lineTo(pos[j][0], pos[j][1]); fb.g.stroke(); }
        inst.nodes.forEach((n, i) => {
          const [x, y] = pos[i], r = 5 + n.flash * 4;
          if (n.flash > 0.05) fb.ring(x - r - 6, y - r - 6, (r + 6) * 2, rgba(pal.accent, n.flash * 0.6), 1);
          fb.disc(x - r, y - r, r * 2, n.flash > 0.2 ? pal.accent : rgba(pal.accent, 0.6));
          fb.ttf(R.noteName(48 + SC[n.deg % 7] + 12 * Math.floor(n.deg / 7)), x + 8, y - 6, 10, rgba(pal.ink, 0.8));
        });
        fb.ttf('PITCH · PHASE · ENVELOPE / TOUCH TO PLAY', 326, 236, 11, rgba(pal.accent, 0.9), { spacing: 0.6 });
        fb.ttf(`PK ${Math.round(inst.pk * 100)}%`, 326, 256, 12, rgba(pal.ink, 0.75)); S.meter(fb, pal, 375, 247, 150, 6, inst.pk * 3);
        fb.ttf(`STEP ${inst.step % 64}`, 612, 256, 12, rgba(pal.ink, 0.75), { align: 'right' });
        fb.ttf('Pads play • F3 starts the generator', 326, 276, 12, rgba(pal.ink, 0.85), { family: 'sans' });
        fb.ttf('L/R: cohesion / pitch spread   U/D: scene   F2: pads', 326, 294, 11, rgba(pal.ink, 0.6), { family: 'sans', maxW: 280 });
      },
    }),
  });

  function ensure(inst) { if (!inst.fx && A.ctx) { inst.fx = A.insertFx(inst.ch); inst.fx.set({ chorus: 0.4, delay: 0.25, dtime: 0.31, fb: 0.4, verb: 0.55 }); } }
  function note(inst, t, midi, vel) {
    const bright = inst.c[2].value;
    return A.fmVoice(inst.fx.in, t, A.midiHz(midi), vel, { rc: 1, r1: 1 + Math.round(bright * 2), r2: 3.5, i1: 0.4 + bright * 1.8, i2: 0.2 + bright, decay: 1.6, atk: 0.006, level: 0.42 });
  }
})();
