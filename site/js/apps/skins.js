/* Skins: a drum machine and step sequencer with eight voices, each with its own lane length (so lanes can
   drift against each other), accents, ratchets and per-step chance. The screen is ported from the app; the
   drum voices are synthesised for this page. */
(() => {
  'use strict';
  const R = window.ROVE, A = R.audio, { c565, blend } = R;
  const BG = c565(4, 3, 5), PANEL = c565(9, 7, 11), DIM = c565(18, 26, 24), INK = '#FAA078', W = '#FFFFFF', K = '#000000', CELL = c565(12, 10, 14), CELL2 = c565(12, 10, 15);
  const NAMES = ['Kick', 'Snare', 'Clap', 'Closed hat', 'Open hat', 'Tom', 'Rim', 'Metal'], DRUM = ['kick', 'snare', 'clap', 'chat', 'ohat', 'tomM', 'rim', 'crash'];
  const COLORS = ['#F0645F', '#FAA53C', '#FADC50', '#6EDC64', '#50D2C8', '#5A96F5', '#AA78F5', '#C86EE6'];
  const VIEWS = ['STEP', 'PLAY', 'SOUND'], FIELDS = ['Accent', 'Ratchet', 'Chance'];
  const KITS = [
    { name: 'Boom', bpm: 120, steps: [[0, 9], [4, 12], [12], [0, 2, 4, 6, 8, 10, 11, 13], [14], [6, 13], [2, 9], []] },
    { name: 'Dust', bpm: 92, steps: [[0, 7, 10], [4, 12], [], [0, 2, 4, 6, 8, 10, 12, 14], [15], [], [3, 11], [0]] },
    { name: 'Chrome', bpm: 132, steps: [[0, 4, 8, 12], [4, 12], [4, 12], [2, 6, 10, 14], [7, 15], [11], [1, 5, 9, 13], []] },
  ];

  R.registerApp({
    id: 'skins', name: 'Skins', category: 'instrument',
    desc: 'A drum machine and step sequencer: eight voices with their own lane lengths, accents, ratchets and chance.',
    create(os) {
      const ch = A.channel('Skins');
      const mk = (k) => KITS[k].steps.map((on) => ({ length: 16, mute: false, tune: 1, decay: 1, level: 1, steps: Array.from({ length: 16 }, (_, s) => ({ on: on.includes(s), accent: on.includes(s) && s % 4 === 0, ratchet: 1, chance: 1 })) }));
      const inst = { view: 'full', ch, kit: 0, lanes: mk(0), v: 0, voice: 0, field: 0, held: null, run: false, step: 0, flash: Array(8).fill(0), row: 0 };
      const ensure = () => { if (!inst.bus && A.ctx) { inst.bus = A.gain(1); inst.bus.connect(ch.input); ch.setSend(0.14); } };
      const hit = (v, t, vel, ratchet = 1, gap = 0.05) => { const l = inst.lanes[v]; for (let r = 0; r < ratchet; r++) A.drum(DRUM[v], inst.bus, t + r * gap, Math.min(1, vel) * l.level * 0.9, l.tune, l.decay); inst.flash[v] = 1; };
      const toggleRun = () => {
        if (!A.ctx) return; ensure();
        if (inst.run) { inst.sched.stop(); inst.run = false; ch.active = false; return; }
        inst.run = true; ch.active = true;
        inst.sched = new A.Scheduler(() => 60 / KITS[inst.kit].bpm / 4, (s, t) => {
          inst.step = s; const dur = 60 / KITS[inst.kit].bpm / 4;
          inst.lanes.forEach((l, v) => { const st = l.steps[s % l.length]; if (st.on && !l.mute && Math.random() <= st.chance) hit(v, t, st.accent ? 1 : 0.65, st.ratchet, dur / st.ratchet); });
        }); inst.sched.start();
      };
      inst.f2 = () => { inst.v = (inst.v + 1) % 3; };
      inst.running = () => inst.run; inst.toggleRunning = toggleRun; inst.transportAction = () => (inst.run ? 'STOP' : 'PLAY');
      inst.enter = () => ensure();
      inst.tick = (inp) => {
        inst.flash = inst.flash.map((f) => Math.max(0, f - 0.08));
        if (inp.navY) inst.voice = Math.max(0, Math.min(7, inst.voice + Math.sign(inp.navY)));
        if (inp.select) toggleRun();
        const lane = inst.lanes[inst.voice];
        if (inst.v === 0) {
          inp.down.forEach((d, p) => { if (d) { const s = lane.steps[p]; if (inst.held == null) { s.on = !s.on; if (!s.on) { s.accent = false; s.ratchet = 1; s.chance = 1; } else if (A.ctx) { ensure(); hit(inst.voice, A.T() + 0.002, 0.8); } } inst.held = p; } });
          inp.up.forEach((u, p) => { if (u && inst.held === p) inst.held = null; });
          if (inp.navX) {
            if (inst.held != null) { const s = lane.steps[inst.held]; if (!s.on) s.on = true; const d = Math.sign(inp.navX); if (inst.field === 0) s.accent = !s.accent; else if (inst.field === 1) s.ratchet = Math.max(1, Math.min(4, s.ratchet + d)); else s.chance = Math.max(0.1, Math.min(1, +(s.chance + d * 0.1).toFixed(1))); }
            else lane.length = Math.max(1, Math.min(16, lane.length + Math.sign(inp.navX)));
          }
          if (inp.r1Edge && inst.held != null) inst.field = (inst.field + 1) % FIELDS.length;
        } else if (inst.v === 1) {
          inp.down.forEach((d, p) => { if (!d) return; if (p < 8) { if (A.ctx) { ensure(); hit(p, A.T() + 0.002, 0.9); } } else inst.lanes[p - 8].mute = !inst.lanes[p - 8].mute; });
        } else {
          inp.down.forEach((d, p) => { if (d && p < 8) inst.voice = p; });
          if (inp.navX) { const k = ['tune', 'decay', 'level'][inst.row], d = Math.sign(inp.navX) * (k === 'level' ? 0.05 : 0.05); lane[k] = Math.max(0.4, Math.min(k === 'level' ? 1.4 : 2, lane[k] + d)); }
          if (inp.r1Edge) inst.row = (inst.row + 1) % 3;
        }
        if (inp.f[3] === false) { /* mixer handled by the shell */ }
      };
      inst.draw = (fb) => {
        fb.clear(BG); fb.rect(0, 0, 640, 30, PANEL); fb.ktext('Skins', 12, 7, 'medium', W);
        VIEWS.forEach((v, i) => { const x = 80 + i * 64, sel = i === inst.v; fb.roundRect(x, 6, 60, 18, 6, sel ? INK : CELL); fb.ktext(v, x + 30, 9, 'small', sel ? K : DIM, 0); });
        fb.ktext(`${KITS[inst.kit].name}  ${KITS[inst.kit].bpm} bpm${inst.run ? '  playing' : ''}`, 630, 9, 'small', DIM, 1);
        const gx = 96, gy = 38, cw = 21, ch2 = 20;
        for (let v = 0; v < 8; v++) {
          const y = gy + v * (ch2 + 4), col = COLORS[v], sel = v === inst.voice, l = inst.lanes[v];
          fb.roundRect(8, y, 84, ch2, 5, sel ? blend(col, BG, 0.3) : blend(PANEL, col, inst.flash[v] * 0.6)); fb.ktext(NAMES[v], 14, y + 4, 'small', sel ? K : l.mute ? DIM : W);
          const pos = inst.run ? inst.step % l.length : -1;
          for (let s = 0; s < 16; s++) {
            const x = gx + s * cw + Math.floor(s / 4) * 3, d = l.steps[s], inside = s < l.length;
            let c = !inside ? BG : d.on ? (d.accent ? blend(col, W, 0.3) : col) : PANEL; if (pos === s) c = blend(c, W, 0.5);
            fb.roundRect(x, y, cw - 3, ch2, 3, c);
            if (d.on && d.ratchet > 1) fb.ktext(String(d.ratchet), x + 9, y + 4, 'small', K, 0); else if (d.on && d.chance < 1) fb.disc(x + 6, y + 7, 6, K);
          }
        }
        const py = 236; fb.roundRect(8, py, 624, 98, 8, PANEL);
        const lane = inst.lanes[inst.voice];
        if (inst.v === 0) {
          if (inst.held != null) {
            const s = lane.steps[inst.held]; fb.ktext(`${NAMES[inst.voice]} step ${inst.held + 1}`, 16, py + 6, 'small', W);
            FIELDS.forEach((f, i) => { const x = 16 + i * 122, y = py + 24, sel = i === inst.field; fb.roundRect(x, y, 116, 30, 6, sel ? blend(PANEL, W, 0.18) : CELL2); fb.ktext(f, x + 6, y + 3, 'small', sel ? W : DIM); fb.ktext([s.accent ? 'on' : 'off', `x${s.ratchet}`, `${Math.round(s.chance * 100)}%`][i], x + 6, y + 16, 'small', INK); });
            fb.ktext('R1: next field   left/right: change it', 16, py + 70, 'small', DIM);
          } else paragraph(fb, `${NAMES[inst.voice]}: tap pads to set steps; hold a step and use left/right to accent, ratchet or set its chance (R1 picks the field). Up/down: voice. Left/right: this lane's length (${lane.length} steps). SELECT: play/stop.`, 16, py + 8, 600);
        } else if (inst.v === 1) paragraph(fb, 'Pads 1-8 play the voices; pads 9-16 mute or unmute them. Up/down picks a voice; SELECT plays and stops the pattern.', 16, py + 8, 600);
        else ['tune', 'decay', 'level'].forEach((k, i) => { const x = 16 + i * 101, y = py + 8, sel = i === inst.row; fb.roundRect(x, y, 96, 40, 6, sel ? blend(PANEL, W, 0.18) : CELL2); fb.ktext(k[0].toUpperCase() + k.slice(1), x + 6, y + 4, 'small', sel ? W : DIM); fb.ktext(`${Math.round(lane[k] * 100)}%`, x + 6, y + 22, 'small', INK); });
        fb.rect(0, 338, 640, 22, PANEL); fb.ktext('F2: view   pads 1-8 on SOUND pick a voice   F3/SELECT: play', 12, 343, 'small', DIM);
      };
      const paragraph = (fb, s, x, y, max) => { const cw = 6, lh = 15, mc = Math.floor(max / cw); let line = '', yy = y; s.split(' ').forEach((w) => { if (line && line.length + 1 + w.length > mc) { fb.ktext(line, x, yy, 'small', DIM); yy += lh; line = ''; } line += (line ? ' ' : '') + w; }); fb.ktext(line, x, yy, 'small', DIM); };
      inst.ledStates = () => Array.from({ length: 16 }, (_, p) => (inst.v === 0 ? (inst.lanes[inst.voice].steps[p].on ? 3 : 0) : inst.v === 1 ? (p < 8 ? 3 : inst.lanes[p - 8].mute ? 0 : 1) : p < 8 ? (p === inst.voice ? 3 : 1) : 0));
      inst.leave = () => { /* the pattern keeps playing when you leave, like a real drum machine */ };
      inst.speak = () => `Skins. ${VIEWS[inst.v]} view. Voice ${NAMES[inst.voice]}.`;
      return inst;
    },
  });
})();
