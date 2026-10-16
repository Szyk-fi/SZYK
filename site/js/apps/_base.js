/* Shared scaffolding for apps built on the play kit. An app supplies its controls, its palette,
   how a pad becomes a sound, and what to draw in the right-hand panel; the kit does the rest. */
(() => {
  'use strict';
  const R = window.ROVE, A = R.audio;
  const { rank, ctl } = R;
  const NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
  const noteName = (m) => NAMES[((m % 12) + 12) % 12] + (Math.floor(m / 12) - 1);
  const SCALES = {
    major: [0, 2, 4, 5, 7, 9, 11], minor: [0, 2, 3, 5, 7, 8, 10], dorian: [0, 2, 3, 5, 7, 9, 10], pent: [0, 2, 4, 7, 9], minpent: [0, 3, 5, 7, 10],
    chromatic: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11], whole: [0, 2, 4, 6, 8, 10], lydian: [0, 2, 4, 6, 7, 9, 11], phrygian: [0, 1, 3, 5, 7, 8, 10],
  };
  /* pad index -> MIDI note. 'chromatic': rank r is base + r. 'scale': rank r is the r-th degree of a scale. */
  const padNote = (p, o) => {
    const rk = rank(p);
    if (o.layout === 'scale') { const s = SCALES[o.scale || 'major']; return o.base + 12 * Math.floor(rk / s.length) + s[rk % s.length] + (o.transpose || 0); }
    return o.base + rk + (o.transpose || 0);
  };
  const isRoot = (p, o) => { const m = padNote(p, o) - (o.transpose || 0); return ((m - o.base) % 12 + 12) % 12 === 0 || (o.rootClass != null && (m % 12) === o.rootClass); };

  const HINT_PLAY = 'L1 + STICK SETS DIAL  ·  ◀▶ TURN  ·  F2 PAD LAYER  ·  R1 MENU';
  const HINT_CONTROLS = 'PAD PICKS A DIAL  ·  WIGGLE STICK OR HAND TO BIND  ·  ◀▶ TURN  ·  R1 MENU';
  const HINT_MENU = '↑↓ ROW  ·  ◀▶ VALUE (HOLD: FASTER)  ·  SELECT OPEN  ·  R1 PLAY VIEW';

  /* spec: { id, name, channel, pal, kit(inst) -> cfg, noteOn(inst, pad, vel, t) -> voice|null, noteOff?, padLabel?, padState?,
             drawRight(fb, inst), line?(inst), footer?(inst), enter?, leave?, tick?(inst, inp), transport?(inst), speak?(inst),
             drawTitle?(fb, inst) }  */
  R.playApp = (spec) => (os) => {
    const ch = A.channel(spec.channel || spec.name);
    const inst = { spec, os, ch, voices: new Map(), t0: performance.now(), frame: 0, data: {} };
    inst.kit = new R.PlayKit(spec.kit(inst));
    const pal = spec.pal;

    const releaseAll = () => { inst.voices.forEach((v) => v && v.stop && v.stop(A.T())); inst.voices.clear(); };
    inst.padLabelNative = (p) => (spec.padLabel ? spec.padLabel(inst, p) : '');
    inst.padStateNative = (p) => {
      if (inst.voices.has(p)) return 3;
      return spec.padState ? spec.padState(inst, p) : 0;
    };
    inst.enter = () => { ch.active = true; if (spec.enter) spec.enter(inst); };
    inst.leave = () => { releaseAll(); if (spec.leave) spec.leave(inst); if (!(inst.running && inst.running() === true)) ch.active = false; };
    inst.tick = (inp) => {
      inst.frame++;
      inst.inp = inp;
      inst.kit.tick(inp, (p, down, vel) => {
        if (!A.ctx) return;
        if (down) { const old = inst.voices.get(p); if (old) old.stop(A.T()); const v = spec.noteOn(inst, p, vel || 1, A.T() + 0.003); if (v) inst.voices.set(p, v); }
        else { const v = inst.voices.get(p); if (v) { v.stop(A.T()); inst.voices.delete(p); } }
      });
      if (inst.kit.menu) releaseAll();
      if (spec.tick) spec.tick(inst, inp);
    };
    inst.view = 'slint';
    inst.draw = (fb) => {
      const S = R.slint;
      S.chrome(fb, pal, spec.title || spec.name, spec.crumb || 'INSTRUMENT');
      if (inst.kit.menu) S.menu(fb, inst.kit, pal);
      else S.column(fb, inst.kit, pal, {
        w: spec.colW,
        padState: (p) => inst.kit.padState(p, inst.padStateNative),
        padLabel: (p) => inst.kit.padLabel(p, inst.padLabelNative),
        line: () => (spec.line ? spec.line(inst) : 'Play'),
      });
      spec.drawRight(fb, inst, pal);
    };
    inst.pal = pal;
    inst.hint = () => (inst.kit.menu ? HINT_MENU : inst.kit.layer.kind === 'controls' ? HINT_CONTROLS : HINT_PLAY);
    inst.gridModeLabel = () => inst.kit.layer.label;
    inst.f2 = () => inst.kit.nextLayer();
    inst.transportAction = () => (spec.transport ? spec.transport(inst).label() : '');
    inst.running = () => (spec.transport ? spec.transport(inst).running() : null);
    inst.toggleRunning = () => { if (spec.transport) spec.transport(inst).toggle(); };
    inst.ledStates = () => Array.from({ length: 16 }, (_, p) => inst.kit.padState(p, inst.padStateNative));
    inst.speak = () => {
      const k = inst.kit; if (k.menu) { const c = k.menuControls[k.menuSel]; return `Menu. ${c.label}: ${c.text}`; }
      const c = k.focusedControl(); return `${spec.name}. ${c.label}: ${c.text}. Pad layer ${k.layer.label}.`;
    };
    inst.control = (label) => inst.kit.menuControls.find((c) => c.label === label);
    return inst;
  };

  R.noteName = noteName; R.padNote = padNote; R.isRoot = isRoot; R.SCALES = SCALES;
  R.fmt = {
    pct: (v) => `${Math.round(v * 100)}%`, hz: (v) => (v >= 1000 ? `${(v / 1000).toFixed(2)} kHz`.replace(/(\.\d)0 /, '$1 ') : `${Math.round(v)} Hz`),
    ms: (v) => (v >= 1000 ? `${(v / 1000).toFixed(2)} s` : `${Math.round(v)} ms`), st: (v) => `${v >= 0 ? '+' : ''}${v.toFixed(v % 1 ? 1 : 0)} st`,
    d2: (v) => v.toFixed(2), d1: (v) => v.toFixed(1), db: (v) => `${v.toFixed(1)} dB`,
  };
  R.c = (r, g, b) => R.c565(r, g, b);
})();
