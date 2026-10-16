/* Turing Machine: a shift-register random sequencer. Each clock the register moves one place and the bit that
   falls off is either recycled or replaced, so a loop can be locked, left random, or evolve. The register
   is the pitch (quantised to a pentatonic scale) and the lowest bit is the pulse. */
(() => {
  'use strict';
  const R = window.ROVE, A = R.audio, { ctl, fmt, rgba } = R, S = R.slint;
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const PAL = S.pal('#EBE8DE', '#232E3A', '#286C9C', '#657887');
  const SCALE = [0, 2, 4, 7, 9];
  const THROWS = ['WRITE 1', 'WRITE 0', 'LOCK', 'DBL LOCK', 'RANDOM', 'FAST', 'SLOW', 'MUTE CV'];
  R.registerApp({
    id: 'turing_machine', name: 'Turing Machine', category: 'sequencer',
    desc: 'A shift-register random sequencer: lock a loop, let it drift, or let it evolve.',
    create: R.playApp({
      name: 'Turing Machine', title: 'Turing Machine', pal: PAL, crumb: 'RANDOM SEQUENCER',
      kit(inst) {
        const c = [
          ctl.f('Locks', 0, 1, 0.5, { fmt: (v) => `${Math.round(v * 100)}%${Math.abs(v - 0.5) < 0.04 ? ' (random)' : v > 0.96 ? ' (locked)' : ''}` }), ctl.f('Rate', 0.5, 16, 4, { fmt: (v) => `${v.toFixed(1)} Hz`, log: true }),
          ctl.int('Length', 2, 16, 16), ctl.f('Scale', 0, 1, 1, { fmt: fmt.pct }),
        ];
        inst.c = c; inst.reg = Array.from({ length: 16 }, () => (Math.random() < 0.5 ? 1 : 0)); inst.m = {}; inst.acc = 0; inst.cv = 0; inst.pulse = 0; inst.head = 0; inst.keep = 0; inst.clock = 0;
        const h = (k) => ({ label: k, control: 0, to: 0, hold: (d) => { inst.m[k] = d; } });
        return { controls: c, hero: [[0, 1], [2, 3]], native: false, throws: THROWS.map(h), routes: { sx: 0, sy: 3, hl: 1 } };
      },
      padLabel: () => '', padState: () => 0,
      line: (inst) => inst.reg.slice(0, inst.c[2].value).join('') + ' random',
      noteOn() { return null; },
      enter(inst) { ensure(inst); inst.on = true; },
      leave(inst) { inst.on = false; },
      tick(inst, inp) {
        if (!inst.on || !A.ctx) return;
        const m = inst.m, rate = inst.c[1].value * (m.FAST ? 2 : 1) * (m.SLOW ? 0.5 : 1), len = inst.c[2].value;
        inst.acc += inp.dt * rate;
        while (inst.acc >= 1) {
          inst.acc -= 1; let lock = inst.c[0].value; if (m.RANDOM) lock = 0.5; if (m.LOCK) lock = 1; if (m['DBL LOCK']) lock = 1;
          const out = inst.reg[len - 1]; let bit;
          if (m['WRITE 1']) bit = 1; else if (m['WRITE 0']) bit = 0;
          else if (lock >= 0.5) bit = Math.random() < (lock - 0.5) * 2 ? out : Math.random() < 0.5 ? 1 : 0; else bit = Math.random() < (0.5 - lock) * 2 ? 1 - out : Math.random() < 0.5 ? 1 : 0;
          inst.keep = Math.abs(lock - 0.5) * 2;
          for (let i = len - 1; i > 0; i--) inst.reg[i] = inst.reg[i - 1]; inst.reg[0] = bit; inst.clock++;
          let val = 0; for (let i = 0; i < Math.min(8, len); i++) val = val * 2 + inst.reg[i]; inst.cv = val / (2 ** Math.min(8, len) - 1); inst.pulse = inst.reg[0];
          if (inst.pulse && !m['MUTE CV']) {
            const idx = Math.round(inst.cv * (inst.c[3].value * 10 + 2)), midi = 48 + SCALE[idx % 5] + 12 * Math.floor(idx / 5);
            A.pluckVoice(inst.bus, A.T() + 0.01, A.midiHz(midi), 0.85, { damp: 0.3, t60: 0.9 + (m['DBL LOCK'] ? 1 : 0), level: 0.9 }); inst.flash = 1;
          }
        }
        inst.flash = (inst.flash || 0) * 0.9;
      },
      drawRight(fb, inst, pal) {
        S.card(fb, pal, 'RANDOM / SHIFT REGISTER');
        fb.ttf('REGISTER', 326, 90, 12, rgba(pal.ink, 0.75), { spacing: 1 });
        for (let i = 0; i < 16; i++) {
          const x = 326 + (i % 4) * 72, y = 100 + Math.floor(i / 4) * 34, on = inst.reg[i] && i < inst.c[2].value;
          fb.rr(x, y, 66, 28, 3, on ? pal.accent : rgba(pal.ink, i < inst.c[2].value ? 0.13 : 0.05), i === 0 ? pal.ink : null);
          fb.ttf(String(i + 1), x + 4, y + 13, 11, on ? pal.paper : rgba(pal.ink, 0.7));
        }
        fb.disc(326, 253, 9, inst.flash > 0.3 ? pal.accent : rgba(pal.ink, 0.3)); fb.ttf(`pulse   cv ${Math.round(inst.cv * 100)}%`, 342, 262, 12, rgba(pal.ink, 0.8));
        S.meter(fb, pal, 326, 272, 286, 6, 0); fb.ttf(`keep probability ${Math.round(inst.keep * 100)}%`, 326, 296, 12, rgba(pal.ink, 0.72));
      },
    }),
  });
  function ensure(inst) { if (!inst.bus && A.ctx) { inst.bus = A.gain(1); inst.bus.connect(inst.ch.input); inst.ch.setSend(0.3); } }
})();
