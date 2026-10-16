/* Pulsar: a beat generator and performance machine. Pick a genre, generate a pattern bank (A-H), and play
   the drum lanes and mutes from the pads while a DJ filter, swing, fatten and room shape the mix. The lane
   names and pad map follow the real app; the drum voices are synthesised for this page. */
(() => {
  'use strict';
  const R = window.ROVE, A = R.audio, { ctl, fmt, rgba } = R, S = R.slint;
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const PAL = S.pal('#1C1014', '#F6E7EA', '#FF5D73', '#9A6F78');
  const LANES = ['KK', 'SN', 'CP', 'CH', 'OH', 'P1', 'P2', 'CR'], VOICE = ['kick', 'snare', 'clap', 'chat', 'ohat', 'conga', 'rim', 'crash'];
  const BANKS = 'ABCDEFGH';
  const GENRES = [['House', 124], ['Techno', 132], ['Hip-hop', 92], ['Breaks', 135], ['Electro', 128]];

  // deterministic-ish generator: base grids per genre, then random garnish
  function generate(genre) {
    const g = Array.from({ length: 8 }, () => Array(16).fill(0));
    const on = (l, steps, v = 1) => steps.forEach((s) => { g[l][s] = v; });
    const maybe = (l, steps, p, v) => steps.forEach((s) => { if (Math.random() < p) g[l][s] = v; });
    if (genre === 0) { on(0, [0, 4, 8, 12]); on(2, [4, 12]); on(3, [2, 6, 10, 14], 0.8); on(4, [14], 0.7); maybe(3, [1, 3, 5, 7, 9, 11, 13, 15], 0.25, 0.35); maybe(5, [3, 7, 11, 13], 0.4, 0.7); maybe(6, [6, 9, 14], 0.4, 0.6); }
    else if (genre === 1) { on(0, [0, 4, 8, 12]); on(2, [4, 12], 0.8); on(3, [2, 6, 10, 14], 0.7); maybe(3, [1, 3, 5, 7, 9, 11, 13, 15], 0.3, 0.4); maybe(5, [3, 5, 9, 11, 15], 0.5, 0.7); maybe(4, [6, 14], 0.5, 0.6); on(7, [0], 0.5); }
    else if (genre === 2) { on(0, [0, 7, 10]); maybe(0, [2, 13], 0.5, 0.8); on(1, [4, 12]); on(3, [0, 2, 4, 6, 8, 10, 12, 14], 0.6); maybe(6, [3, 11, 15], 0.4, 0.5); maybe(4, [14], 0.5, 0.6); }
    else if (genre === 3) { on(0, [0, 10]); maybe(0, [3, 7, 13], 0.5, 0.8); on(1, [4, 12]); maybe(1, [7, 9, 15], 0.5, 0.5); on(3, [0, 2, 4, 6, 8, 10, 12, 14], 0.6); maybe(5, [1, 6, 11], 0.5, 0.6); on(7, [0], 0.4); }
    else { on(0, [0, 3, 8, 11]); on(1, [4, 12]); on(2, [12]); on(3, [2, 6, 10, 14], 0.7); maybe(5, [5, 7, 13, 15], 0.5, 0.7); maybe(6, [2, 9], 0.5, 0.6); }
    return g;
  }
  const variation = (base) => base.map((row, l) => row.map((v, s) => (l === 0 && s % 4 === 0 ? v : Math.random() < 0.12 ? (v ? 0 : 0.6) : v)));

  R.registerApp({
    id: 'pulsar', name: 'Pulsar', category: 'sequencer',
    desc: 'A beat generator and performance machine: eight drum lanes, a pattern bank, and pads that play the lanes and mute them.',
    create: R.playApp({
      name: 'Pulsar', pal: PAL, crumb: 'BEAT MACHINE',
      kit(inst) {
        const c = [
          ctl.f('DJ Filter', -1, 1, 0, { fmt: (v) => (Math.abs(v) < 0.03 ? 'off' : v < 0 ? `LP ${Math.round(-v * 100)}%` : `HP ${Math.round(v * 100)}%`), onChange: () => fx(inst) }),
          ctl.f('Swing', 0.5, 0.75, 0.54, { fmt: (v) => `${Math.round(v * 100)}%` }), ctl.f('Fatten', 0, 1, 0.2, { fmt: fmt.pct, onChange: () => fx(inst) }), ctl.f('Room', 0, 1, 0.3, { fmt: fmt.pct, onChange: () => fx(inst) }),
          ctl.choice('Genre', GENRES.map((g) => g[0]), 0, { onChange: (v) => { inst.c[5].base = GENRES[v][1]; newBank(inst); } }), ctl.f('Tempo', 70, 170, 124, { fmt: (v) => `${Math.round(v)} BPM` }),
          ctl.int('Pattern', 0, 7, 0, { fmt: (v) => BANKS[v] }),
        ];
        inst.c = c; inst.run = false; inst.step = 0; inst.muted = Array(8).fill(false); inst.flash = Array(8).fill(0); inst.peak = 0; inst.fill = false;
        inst.bank = null; setTimeout(() => newBank(inst), 0);
        return { controls: c, hero: [[0, 1], [2, 3], [4, 5]], layers: ['PERFORM'], browse: 6, routes: { sx: 0, sy: 2, hl: 3, hr: 1 } };
      },
      padLabel: (inst, p) => (p < 8 ? LANES[p] : `${LANES[p - 8]} mute`),
      padState: (inst, p) => (p < 8 ? (inst.bank && inst.bank[inst.c[6].value][p].some((v) => v) ? 3 : 0) : inst.muted[p - 8] ? 0 : 3),
      line: (inst) => (inst.run ? `${BANKS[inst.c[6].value]} › bar ${Math.floor(inst.step / 16) % 2 + 1} › ${LANES[inst.lastLane ?? 0]}` : `${BANKS[inst.c[6].value]} › bar 1 › KK`),
      enter(inst) { ensure(inst); },
      noteOn(inst, p, vel, t) {
        ensure(inst);
        if (p < 8) { hit(inst, p, t, 1); inst.lastLane = p; } else { inst.muted[p - 8] = !inst.muted[p - 8]; }
        return null;
      },
      transport: (inst) => ({ label: () => (inst.run ? 'STOP' : 'PLAY'), running: () => inst.run, toggle: () => toggle(inst) }),
      tick(inst) { inst.flash = inst.flash.map((f) => f * 0.9); inst.peak *= 0.94; },
      drawRight(fb, inst, pal) {
        const g = GENRES[inst.c[4].value], pat = inst.bank ? inst.bank[inst.c[6].value] : null;
        S.card(fb, pal, 'BEAT / PATTERN BANK');
        fb.ttf(g[0], 326, 90, 14, pal.accent, { family: 'sans', weight: 700 }); fb.ttf(`${Math.round(inst.c[5].value)} BPM`, 380, 90, 12, rgba(pal.ink, 0.72));
        BANKS.split('').forEach((b, i) => { const x = 326 + i * 36; fb.rr(x, 98, 30, 14, 2, i === inst.c[6].value ? rgba(pal.ink, 0.15) : null, i === inst.c[6].value ? pal.ink : null); fb.ttf(b, x + 15, 109, 12, i === inst.c[6].value ? pal.ink : rgba(pal.ink, 0.5), { align: 'center' }); });
        const gx = 348, gy = 120, cw = 16.2, ch = 13.4, ph = inst.run ? inst.step % 16 : -1;
        LANES.forEach((l, r) => {
          const y = gy + r * (ch + 1.4);
          fb.ttf(l, 326, y + 11, 11, inst.muted[r] ? rgba(pal.ink, 0.3) : r === (inst.lastLane ?? -1) ? pal.accent : rgba(pal.ink, 0.8), { weight: r === (inst.lastLane ?? -1) ? 700 : 400 });
          for (let s = 0; s < 16; s++) {
            const v = pat ? pat[r][s] : 0, x = gx + s * cw, beat = s % 4 === 0;
            fb.rr(x, y, cw - 1.5, ch, 2, v ? rgba(inst.muted[r] ? pal.ink : pal.accent, 0.35 + v * 0.65 * (inst.muted[r] ? 0.2 : 1)) : rgba(pal.ink, beat ? 0.1 : 0.05));
          }
          if (inst.flash[r] > 0.1) fb.rr(326 + 280, y, 6, ch, 2, rgba(pal.accent, inst.flash[r]));
        });
        if (ph >= 0) fb.rect(gx + ph * cw - 1, gy - 2, cw + 1, 8 * (ch + 1.4) + 2, rgba(pal.ink, 0.1));
        fb.rect(326, 246, 286, 3, rgba(pal.ink, 0.12)); fb.rect(326, 246, 286 * ((inst.step % 32) / 32), 3, pal.accent);
        fb.ttf(`${inst.run ? 'PERFORM' : 'STOPPED'} · BAR ${Math.floor(inst.step / 16) % 2 + 1}/2 · SWING ${Math.round(inst.c[1].value * 100)}%`, 326, 268, 12, rgba(pal.ink, 0.85));
        S.meter(fb, pal, 326, 276, 286, 4, inst.peak * 3);
        fb.ttf(inst.run ? 'Playing. Top pads hit lanes; lower pads mute.' : `${g[0]} beat ready. F3 to play, Generate for another.`, 326, 296, 12, pal.accent, { family: 'sans', maxW: 286 });
      },
    }),
  });

  function ensure(inst) {
    if (inst.bus || !A.ctx) return;
    const c = A.ctx;
    inst.bus = A.gain(1); inst.hp = A.filt('highpass', 20, 0.7); inst.lp = A.filt('lowpass', 20000, 0.7);
    inst.sat = A.shaper(4, false); inst.satG = A.gain(0); inst.dry = A.gain(1);
    inst.bus.connect(inst.hp); inst.hp.connect(inst.lp); inst.lp.connect(inst.dry); inst.lp.connect(inst.sat); inst.sat.connect(inst.satG);
    inst.dry.connect(inst.ch.input); inst.satG.connect(inst.ch.input);
    fx(inst);
  }
  function fx(inst) {
    if (!inst.bus) return; const v = inst.c[0].value, t = A.T();
    inst.lp.frequency.setTargetAtTime(v < -0.03 ? 18000 * 0.002 ** -v : 20000, t, 0.05); inst.hp.frequency.setTargetAtTime(v > 0.03 ? 20 + 6000 * v ** 2 : 20, t, 0.05);
    inst.satG.gain.setTargetAtTime(inst.c[2].value * 0.5, t, 0.05); inst.ch.setSend(0.03 + inst.c[3].value * 0.45);
  }
  function newBank(inst) {
    const base = generate(inst.c[4].value); inst.bank = Array.from({ length: 8 }, (_, i) => (i === 0 ? base : variation(base))); inst.kit && inst.kit.flash(`${GENRES[inst.c[4].value][0]} generated`);
  }
  function hit(inst, lane, t, vel) {
    A.drum(VOICE[lane], inst.bus, t, 0.85 * vel); inst.flash[lane] = 1; inst.peak = Math.max(inst.peak, 0.12 * vel);
  }
  function toggle(inst) {
    if (!A.ctx) return; ensure(inst); if (!inst.bank) newBank(inst);
    if (inst.run) { inst.sched.stop(); inst.run = false; inst.ch.active = false; return; }
    inst.run = true; inst.ch.active = true; inst.step = 0;
    inst.sched = new A.Scheduler(() => 60 / inst.c[5].value / 4, (s, t) => {
      const pat = inst.bank[inst.c[6].value], st = s % 16, sw = st % 2 ? (inst.c[1].value - 0.5) * 2 * (60 / inst.c[5].value / 4) : 0;
      inst.step = s;
      LANES.forEach((l, r) => { const v = pat[r][st]; if (v && !inst.muted[r]) hit(inst, r, t + sw, v); });
    });
    inst.sched.start();
  }
})();
