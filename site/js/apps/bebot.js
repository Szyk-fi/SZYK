/* Bebot: a mouse-and-touch XY synth. Hold and drag on the screen: left to right is pitch, bottom to top is
   tone. Pitch can snap to a scale, and the pads play the same notes. Presets, waveform and settings are in the
   top bar. (An independent instrument inspired by touch instruments; not anyone else's engine.) */
(() => {
  'use strict';
  const R = window.ROVE, A = R.audio, { c565, blend } = R;
  const BG = c565(2, 5, 7), TEAL = c565(8, 57, 26), INK = c565(27, 58, 30), DIM = c565(16, 37, 22), BAR = c565(4, 12, 11), LINE_ON = c565(4, 16, 12), LINE_OFF = c565(3, 9, 9), SEL = c565(4, 19, 14);
  const SCALES = ['Pentatonic', 'Major', 'Minor', 'Chromatic'], SC = [[0, 2, 4, 7, 9], [0, 2, 4, 5, 7, 9, 11], [0, 2, 3, 5, 7, 8, 10], [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]];
  const MODES = ['Saw', 'Square', 'Triangle', 'Sine'], OSC = ['sawtooth', 'square', 'triangle', 'sine'], TUNES = ['Snap', 'Free'];
  // preset: name, mode, scale, tune, low, range, glide, bright, echo
  const PRESETS = [['Robot song', 0, 0, 0, 48, 24, 0.12, 0.55, 0.3], ['Glass bot', 3, 0, 0, 60, 24, 0.05, 0.7, 0.45], ['Sub bot', 1, 2, 0, 36, 24, 0.1, 0.3, 0.1], ['Choir bot', 2, 1, 0, 55, 24, 0.2, 0.5, 0.4], ['Chip bot', 1, 0, 0, 60, 24, 0.02, 0.9, 0.05], ['Wail', 0, 3, 1, 48, 36, 0.3, 0.6, 0.35]];
  const LABELS = ['Preset', 'Waveform', 'Scale', 'Tuning', 'Low note', 'Range', 'Glide', 'Brightness', 'Echo', 'Level'];

  R.registerApp({
    id: 'bebot', name: 'Bebot', category: 'instrument',
    desc: 'A mouse-and-touch XY synth: hold and drag for pitch and tone, snapped to a scale. Pads play the same notes.',
    create(os) {
      const ch = A.channel('Bebot'), st = { preset: 0, v: [0, 0, 0, 0, 48, 24, 0.12, 0.55, 0.3, 0.8] };
      const inst = { view: 'full', ch, voice: null, menu: false, sel: 0, ptr: null, pads: new Map(), status: '' };
      const load = (i) => { st.preset = i; const p = PRESETS[i]; st.v = [i, p[1], p[2], p[3], p[4], p[5], p[6], p[7], p[8], st.v[9]]; apply(); };
      const apply = () => { if (inst.bus) { inst.echo.gain.setTargetAtTime(st.v[8] * 0.7, A.T(), 0.05); ch.setLevel(st.v[9] * 1.2); } };
      const value = (i) => [PRESETS[st.preset][0], MODES[st.v[1]], SCALES[st.v[2]], TUNES[st.v[3]], `${Math.round(st.v[4])}`, `${Math.round(st.v[5])} st`, `${Math.round(st.v[6] * 100)}%`, `${Math.round(st.v[7] * 100)}%`, `${Math.round(st.v[8] * 100)}%`, `${Math.round(st.v[9] * 100)}%`][i];
      const adjust = (i, d) => {
        if (i === 0) load((st.preset + d + PRESETS.length) % PRESETS.length); else if (i === 1) st.v[1] = (st.v[1] + d + 4) % 4; else if (i === 2) st.v[2] = (st.v[2] + d + 4) % 4; else if (i === 3) st.v[3] = 1 - st.v[3];
        else if (i === 4) st.v[4] = Math.max(24, Math.min(72, st.v[4] + d)); else if (i === 5) st.v[5] = Math.max(12, Math.min(48, st.v[5] + d * 2)); else st.v[i] = Math.max(0, Math.min(1, st.v[i] + d * 0.05));
        apply();
      };
      const ensure = () => { if (inst.bus || !A.ctx) return; inst.bus = A.gain(1); inst.bus.connect(ch.input); const dl = A.ctx.createDelay(1), fb = A.gain(0.4), em = A.gain(0); dl.delayTime.value = 0.3; inst.bus.connect(dl); dl.connect(fb); fb.connect(dl); dl.connect(em); em.connect(ch.input); inst.echo = em; ch.setSend(0.25); apply(); };
      const snap = (n) => { if (st.v[3]) return n; const sc = SC[st.v[2]]; let best = n, bd = 1e9; for (let o = -1; o <= 6; o++) sc.forEach((s) => { const c = Math.floor(st.v[4] / 12) * 12 + o * 12 + s; if (Math.abs(c - n) < bd) { bd = Math.abs(c - n); best = c; } }); return best; };
      const noteAt = (x) => st.v[4] + (x / 639) * st.v[5];
      const start = (x, y) => {
        ensure(); const t = A.T() + 0.003, o = A.osc(OSC[st.v[1]], 220, t), lp = A.filt('lowpass', 1200, 2), env = A.gain(0), rel = A.gain(1);
        env.gain.setValueAtTime(0, t); env.gain.linearRampToValueAtTime(0.38, t + 0.01); o.connect(lp); lp.connect(env); env.connect(rel); rel.connect(inst.bus); o.start(t); o.stop(t + 120);
        inst.voice = { o, lp, rel }; move(x, y, true);
      };
      const move = (x, y, first) => {
        const v = inst.voice; if (!v) return; const f = A.midiHz(snap(noteAt(x))), t = A.T(), g = first ? 0.001 : Math.max(0.005, st.v[6] * 0.12);
        v.o.frequency.setTargetAtTime(f, t, g); v.lp.frequency.setTargetAtTime(200 + (1 - y / 360) ** 1.6 * 9000 * (0.4 + st.v[7]), t, 0.03); inst.note = snap(noteAt(x));
      };
      const stop = () => { const v = inst.voice; if (v) { A.release(v.rel, [v.o], A.T(), 0.12); inst.voice = null; } };

      inst.enter = () => { ensure(); };
      inst.leave = () => { stop(); inst.pads.forEach((p) => p.stop(A.T())); inst.pads.clear(); };
      inst.tick = (inp) => {
        if (inp.ptr && A.ctx) {
          const p = inp.ptr;
          if (p.type === 'down') {
            if (p.y < 40) { if (p.x < 60) os.home(); else if (p.x < 300) adjust(0, 1); else if (p.x < 405) adjust(1, 1); else if (p.x < 520) adjust(3, 1); else inst.menu = !inst.menu; }
            else if (!inst.menu) { inst.ptr = p; start(p.x, p.y); }
          } else if (p.type === 'move' && inst.ptr) { inst.ptr = p; move(p.x, p.y); } else if (p.type === 'up') { inst.ptr = null; stop(); }
        }
        if (inp.ptrNow && inst.ptr && inst.voice) { inst.ptr = inp.ptrNow; move(inp.ptrNow.x, inp.ptrNow.y); }
        if (inp.r1Edge) inst.menu = !inst.menu;
        if (inst.menu) { if (inp.navY) inst.sel = Math.max(0, Math.min(LABELS.length - 1, inst.sel + Math.sign(inp.navY))); if (inp.navX) adjust(inst.sel, Math.sign(inp.navX)); }
        else { if (inp.navX) adjust(0, Math.sign(inp.navX)); }
        inp.down.forEach((d, p) => { if (d && A.ctx && !inst.menu) { ensure(); const r = R.rank(p), n = snap(st.v[4] + r * (st.v[5] / 16)); inst.pads.set(p, A.pluckVoice(inst.bus, A.T() + 0.003, A.midiHz(n), 0.9, { damp: 0.28, t60: 1.6, level: 0.9 })); inst.note = n; } });
        inp.up.forEach((u, p) => { if (u) inst.pads.delete(p); });
      };
      inst.draw = (fb) => {
        fb.clear(BG); fb.rect(0, 0, 640, 40, BAR);
        [['<', 20], [PRESETS[st.preset][0], 65], [MODES[st.v[1]], 305], [TUNES[st.v[3]], 410], [inst.menu ? 'PLAY' : 'SETTINGS', 530]].forEach(([s, x]) => fb.ktext(s, x, 14, 'medium', INK));
        if (inst.menu) {
          LABELS.forEach((l, i) => { const x = 12 + Math.floor(i / 9) * 320, y = 49 + (i % 9) * 28; if (inst.sel === i) fb.rect(x - 4, y, 310, 28, SEL); fb.ktext(l, x, y + 6, 'medium', DIM); fb.ktext(value(i), x + 290, y + 6, 'medium', TEAL, 1); });
          fb.ktext('up/down: setting   left/right: change   R1 or SETTINGS: back to play', 320, 330, 'small', TEAL, 0); return;
        }
        const sc = SC[st.v[2]];
        for (let i = 0; i <= 24; i++) {
          const x = Math.floor(i * 640 / 24), note = st.v[4] + (i / 24) * st.v[5], sn = snap(note), on = Math.abs(note - sn) < 0.1;
          fb.rect(x, 42, 1, 294, on ? LINE_ON : LINE_OFF);
          if (on && (sn % 12 === 0 || sc.length < 8 && i % 2 === 0)) fb.ktext(R.noteName(Math.round(sn)), x + 3, 322, 'small', on ? TEAL : DIM);
        }
        for (let b = 1; b < 4; b++) fb.rect(0, 42 + Math.floor(b * 294 / 4), 640, 1, LINE_OFF);
        fb.ktext('BEBOT / XY SYNTH', 16, 62, 'medium', TEAL);
        fb.ktext(`${SCALES[st.v[2]]} / ${TUNES[st.v[3]]}   ${Math.round(st.v[4])}-${Math.round(st.v[4] + st.v[5])}`, 16, 86, 'small', DIM);
        fb.ktext('TONE ↑', 16, 110, 'small', DIM);
        if (inst.ptr && inst.voice) { const px = Math.round(inst.ptr.x), py = Math.round(inst.ptr.y); fb.disc(px - 12, py - 12, 24, TEAL); fb.disc(px - 7, py - 7, 14, BG); fb.ktext(R.noteName(Math.round(inst.note)), Math.min(610, px + 18), py - 6, 'small', INK); }
        fb.rect(0, 338, 640, 22, BAR); fb.ktext('HOLD + DRAG    X: PITCH    Y: TONE    RELEASE: STOP', 12, 343, 'small', DIM);
      };
      inst.ledStates = () => Array.from({ length: 16 }, (_, p) => (inst.pads.has(p) ? 3 : R.rank(p) % 5 === 0 ? 1 : 0));
      inst.speak = () => `Bebot. ${PRESETS[st.preset][0]}. Drag on the screen to play, or use the pads.`;
      load(0);
      return inst;
    },
  });
})();
