/* Bloom: an orbital sequencer. A hand sweeps a spiral of notes and an outer ring of gates; whatever it
   crosses plays. The throws (FAST, CRAWL, FLIP, MIRROR, HUSH, BRIGHT, LONG, FAN) bend the motion while held. */
(() => {
  'use strict';
  const R = window.ROVE, A = R.audio, { ctl, fmt, rgba } = R, S = R.slint;
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const PAL = S.pal('#0C1918', '#E7EDDA', '#D8F580', '#203B33');
  const MUTED = '#829B8A', CORAL = '#FF9C79';
  const SCALE = [0, 2, 4, 7, 9];                                      // pentatonic: every node sounds right
  const THROWS = ['FAST', 'CRAWL', 'FLIP', 'FAN', 'MIRROR', 'HUSH', 'BRIGHT', 'LONG'];
  const SHAPES = ['Spiral', 'Bloom', 'Orbit', 'Helix'];

  R.registerApp({
    id: 'bloom', name: 'Bloom', category: 'sequencer',
    desc: 'An orbital sequencer: a sweeping hand crosses a spiral of notes and a ring of gates, and everything it touches plays.',
    create: R.playApp({
      name: 'Bloom', pal: PAL, crumb: '',
      kit(inst) {
        const b = (l) => ctl.bool(l, 0);
        const c = [
          ctl.f('Speed', 0.05, 1.2, 0.25, { fmt: (v) => `${v.toFixed(2)} Hz`, log: true }), ctl.f('Outer Angle Offset', -1, 1, 0, { fmt: (v) => `${v >= 0 ? '+' : ''}${Math.round(v * 100)}%` }),
          ctl.f('Probability', 0, 1, 1, { fmt: fmt.pct }), ctl.f('Dot Angle Offset', -1, 1, 0, { fmt: (v) => `${v >= 0 ? '+' : ''}${Math.round(v * 100)}%` }),
          ...THROWS.map(b), ctl.int('Shape', 0, 3, 0, { fmt: (v) => SHAPES[v], onChange: () => build(inst) }),
        ];
        inst.c = c; inst.run = false; inst.theta = 0; inst.dir = 1; inst.notes = 0; inst.lastHit = 0; inst.hitNode = -1; inst.gates = [];
        build(inst);
        return { controls: c, hero: [[0, 1], [2, 3]], native: false, throws: THROWS.map((n, i) => ({ control: 4 + i, to: 1, label: n })), routes: { sx: 0, sy: 2 } };
      },
      padLabel: () => '', padState: () => 0,
      line: (inst) => (inst.run ? 'Shape running' : 'Play'),
      enter(inst) { ensure(inst); },
      transport: (inst) => ({ label: () => (inst.run ? 'STOP' : 'START'), running: () => inst.run, toggle: () => { inst.run = !inst.run; inst.ch.active = inst.run; } }),
      tick(inst, inp) {
        const th = (n) => inst.c[4 + THROWS.indexOf(n)].value;
        if (!inst.run || !A.ctx) return;
        const speed = inst.c[0].value * (th('FAST') ? 2 : 1) * (th('CRAWL') ? 0.25 : 1), dir = th('MIRROR') ? -1 : 1;
        const prev = inst.theta; inst.theta = (inst.theta + dir * speed * inp.dt * Math.PI * 2) % (Math.PI * 2); if (inst.theta < 0) inst.theta += Math.PI * 2;
        const TAU = Math.PI * 2, nm = (x) => ((x % TAU) + TAU) % TAU, p0 = nm(prev), p1 = nm(inst.theta);
        const crossed = (a) => { a = nm(a); return dir > 0 ? (p0 <= p1 ? a > p0 && a <= p1 : a > p0 || a <= p1) : (p0 >= p1 ? a < p0 && a >= p1 : a < p0 || a >= p1); };
        inst.nodes.forEach((n, i) => {
          if (crossed((n.a + inst.c[3].value * Math.PI) % (Math.PI * 2)) && Math.random() < inst.c[2].value) { n.flash = 1; inst.hitNode = i; inst.lastHit = A.T(); inst.notes++; play(inst, n, th); }
        });
        inst.gates.forEach((g) => { if (crossed((g.a + inst.c[1].value * Math.PI) % (Math.PI * 2)) && Math.random() < inst.c[2].value) { g.flash = 1; A.drum(g.drum, inst.bus, A.T() + 0.005, th('HUSH') ? 0.2 : 0.5); } });
        inst.nodes.forEach((n) => { n.flash *= 0.94; }); inst.gates.forEach((g) => { g.flash *= 0.94; });
      },
      drawRight(fb, inst, pal) {
        fb.ttf(SHAPES[inst.c[12].value], 296, 83, 24, pal.ink, { family: 'sans', weight: 600 });
        fb.ttf('SHAPE', 296, 112, 12, MUTED, { spacing: 1 }); fb.ttf(String(inst.c[12].value + 1).padStart(2, '0'), 296, 142, 30, pal.accent, { family: 'sans', weight: 600 });
        fb.rr(552, 66, 70, 24, 12, pal.accent); fb.ttf(inst.run ? '● LIVE' : 'READY', 587, 83, 12, pal.paper, { align: 'center', weight: 700, spacing: 1 });
        const cx = 456, cy = 196, rr = [28, 56, 84, 112];
        rr.forEach((r) => { fb.g.strokeStyle = rgba(pal.accent, 0.18); fb.g.lineWidth = 1; fb.g.beginPath(); fb.g.arc(cx, cy, r, 0, Math.PI * 2); fb.g.stroke(); });
        // the sweeping hand
        const th = inst.theta, hx = cx + Math.cos(th) * 112, hy = cy + Math.sin(th) * 112; fb.g.strokeStyle = rgba(pal.accent, inst.run ? 0.45 : 0.15); fb.g.beginPath(); fb.g.moveTo(cx, cy); fb.g.lineTo(hx, hy); fb.g.stroke();
        const pos = inst.nodes.map((n) => { const a = n.a + inst.c[3].value * Math.PI; return [cx + Math.cos(a) * n.r, cy + Math.sin(a) * n.r]; });
        fb.g.strokeStyle = rgba(pal.accent, 0.55); fb.g.lineWidth = 1.5; fb.g.beginPath(); pos.forEach(([x, y], i) => (i ? fb.g.lineTo(x, y) : fb.g.moveTo(x, y))); fb.g.stroke();
        inst.nodes.forEach((n, i) => { const [x, y] = pos[i], r = 3 + n.flash * 4; fb.disc(x - r, y - r, r * 2, n.flash > 0.2 ? CORAL : rgba(pal.accent, 0.85)); });
        inst.gates.forEach((g) => { const a = g.a + inst.c[1].value * Math.PI, x = cx + Math.cos(a) * 112, y = cy + Math.sin(a) * 112; fb.rect(x - 5, y - 5, 10, 10, g.flash > 0.2 ? '#FFB02E' : rgba(pal.accent, 0.8)); });
        fb.ttf(`/ ${String(inst.nodes.length).padStart(2, '0')}`, 620, 284, 12, MUTED, { align: 'right' });
        fb.ttf(`${inst.nodes.length} NOTES`, 296, 322 - 8, 12, pal.accent, { spacing: 1 }); fb.ttf(`${inst.gates.length} GATES`, 400, 314, 12, MUTED, { spacing: 1 });
        fb.ttf('● LAST HIT', 620, 314, 12, A.T() - inst.lastHit < 1.5 && inst.run ? CORAL : MUTED, { align: 'right', spacing: 1 });
      },
    }),
  });

  function build(inst) {
    const n = 8; inst.nodes = Array.from({ length: n }, (_, i) => ({ a: i * 2.399963 + inst.c[12].value, r: 22 + i * 11.5, deg: i, flash: 0 }));
    inst.gates = [[0.4, 'kick'], [2.3, 'rim'], [4.4, 'conga']].map(([a, d]) => ({ a: a + inst.c[12].value * 0.5, drum: d, flash: 0 }));
  }
  function ensure(inst) { if (inst.bus || !A.ctx) return; inst.bus = A.gain(1); inst.bus.connect(inst.ch.input); inst.ch.setSend(0.3); }
  function play(inst, n, th) {
    ensure(inst); let deg = th('FLIP') ? 7 - n.deg : n.deg; if (th('FAN')) deg += 2;
    const midi = 55 + SCALE[((deg % 5) + 5) % 5] + 12 * Math.floor(deg / 5), bright = th('BRIGHT') ? 1 : 0, long = th('LONG') ? 2.5 : 1, vel = th('HUSH') ? 0.25 : 0.8;
    A.pluckVoice(inst.bus, A.T() + 0.005, A.midiHz(midi), vel, { damp: 0.35 - bright * 0.25, t60: 1.4 * long, level: 0.9 });
  }
})();
