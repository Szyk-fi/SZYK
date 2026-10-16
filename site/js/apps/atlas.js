/* Atlas: a meta-synth. Seven performance macros and a MORPH steer every parameter of a preset.
   Preset ranges, macro targets and morph end-states are the real ones (assets/atlas/presets);
   the voices are compact Web Audio recipes for each preset's family. */
(() => {
  'use strict';
  const R = window.ROVE, A = R.audio, { ctl, rgba, fmt } = R, S = R.slint;
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const PAL = S.pal('#14191F', '#E7EDF4', '#A5BCE9', '#8994AA');
  const MACROS = ['CHARACTER', 'COLOR', 'MOTION', 'SPACE', 'SHAPE', 'ENERGY', 'TEXTURE'];
  const PRESETS = R.data.atlas;
  const SEQ = [0, 2, 3, 5, 7, 8, 10];                      // pads walk the scale from A3 (C major, so C is the root)
  const midiOf = (p) => { const r = R.rank(p); return 57 + SEQ[r % 7] + 12 * Math.floor(r / 7); };

  /* effective parameter set: defaults, glided toward the B state by MORPH, then pushed by the macros */
  function effective(inst) {
    const pr = PRESETS[inst.preset], morph = inst.c[7].value, out = {};
    pr.params.forEach((q) => {
      const norm = (v) => (q.exp ? Math.log(v / q.min) / Math.log(q.max / q.min) : (v - q.min) / (q.max - q.min));
      let n = norm(q.def);
      if (pr.b[q.id] != null) n += (norm(pr.b[q.id]) - n) * morph;
      MACROS.forEach((m, i) => pr.macros[m].forEach(([id, amt]) => { if (id === q.id) n += amt * inst.c[i].value; }));
      n = clamp(n, 0, 1);
      out[q.id] = q.exp ? q.min * (q.max / q.min) ** n : q.min + n * (q.max - q.min);
    });
    return out;
  }

  const hz = (m) => A.midiHz(m);
  /* ---- one recipe per preset family ---- */
  const RECIPES = {
    init(d, t, f, vel, P) {
      const lp = A.filt('lowpass', P.cutoff, 0.5 + P.res * 12), env = A.gain(0), rel = A.gain(1), drive = A.shaper(P.drive, false);
      const src = [], mix = A.gain(0.3);
      [[-1, 'sawtooth'], [1, 'sawtooth']].forEach(([s, w]) => { const o = A.osc(P.wave > 0.5 ? 'square' : w, f, t); o.detune.value = s * P.detune * 100; o.connect(mix); o.start(t); src.push(o); });
      const sub = A.osc('square', f / 2, t), sg = A.gain(P.sub * 0.5); sub.connect(sg); sg.connect(mix); sub.start(t); src.push(sub);
      lp.frequency.setValueAtTime(clamp(P.cutoff * 2 ** P.envamt, 40, 18000), t); lp.frequency.setTargetAtTime(P.cutoff, t, Math.max(0.01, P.fdecay / 3));
      if (P.wah > 0.01 || P.vib > 0.01) { const l = A.osc('sine', P.rate, t), lg = A.gain(P.wah * 1500); l.connect(lg); lg.connect(lp.detune); const vg = A.gain(P.vib * 100); l.connect(vg); src.forEach((o) => o.detune && vg.connect(o.detune)); l.start(t); src.push(l); }
      env.gain.setValueAtTime(0, t); env.gain.linearRampToValueAtTime(0.5 * vel, t + 0.005); env.gain.setTargetAtTime(0.4 * vel, t + 0.01, 0.3);
      mix.connect(drive); drive.connect(lp); lp.connect(env); env.connect(rel); rel.connect(d);
      return { stop(t2) { A.release(rel, src, t2, 0.25); }, lp };
    },
    glass(d, t, f, vel, P) {
      const src = [], mix = A.gain(0.18), fold = A.shaper(P.fold, true);
      [1, 2, 3, 4, 5, 6].forEach((h, i) => { const o = A.osc('sine', f * h * (1 + i * 0.002), t), g = A.gain(Math.exp(-i * (1.8 - P.pos * 1.6)) * (0.4 + P.pos)); o.connect(g); g.connect(mix); o.start(t); src.push(o); });
      if (P.blend > 0.02) { const n = A.noiseSrc(), bp = A.filt('bandpass', f * 4, 3), ng = A.gain(P.blend * 0.6); n.connect(bp); bp.connect(ng); ng.connect(mix); n.start(t); src.push(n); }
      const lp = A.filt('lowpass', P.cutoff, 0.5 + P.res * 10); lp.frequency.setValueAtTime(clamp(P.cutoff * 2 ** (P.envamt * 0.6), 40, 18000), t); lp.frequency.setTargetAtTime(P.cutoff, t, 0.25);
      const env = A.gain(0), rel = A.gain(1); env.gain.setValueAtTime(0, t); env.gain.linearRampToValueAtTime(0.5 * vel * P.gain, t + 0.03); env.gain.setTargetAtTime(0.35 * vel * P.gain, t + 0.05, 0.6);
      mix.connect(fold); fold.connect(lp); lp.connect(env);
      // modal ring
      const ring = A.gain(P.resmix * 0.9); env.connect(ring);
      [1, 2.76, 5.4, 8.93].forEach((r, i) => { const bp = A.filt('bandpass', clamp(f * (1 + (r - 1) * (1 - P.structure * 0.5)), 60, 14000), 20 * (1 - P.damp * 0.8)); const g = A.gain(0.8 / (i + 1)); ring.connect(bp); bp.connect(g); g.connect(rel); });
      env.connect(rel); rel.connect(d);
      const l = A.osc('sine', P.lfo1rate, t), lg = A.gain(P.lfo1amt * 900); l.connect(lg); lg.connect(lp.detune); l.start(t); src.push(l);
      return { stop(t2) { A.release(rel, src, t2, 0.9); }, lp };
    },
    strings(d, t, f, vel, P) {
      const src = [], mix = A.gain(0.14), lp = A.filt('lowpass', 800 + P.timbre * 6000, 0.7), env = A.gain(0), rel = A.gain(1);
      [-14, -5, 0, 6, 13].forEach((c) => { const o = A.osc(P.morph > 0.5 ? 'square' : 'sawtooth', f, t); o.detune.value = c * (1 + P.drift * 4); o.connect(mix); o.start(t); src.push(o); });
      const sh = A.osc('sawtooth', f * 2 ** (P.gpitch / 12), t), sg = A.gain(P.shadow * 0.1); sh.connect(sg); sg.connect(mix); sh.start(t); src.push(sh);
      env.gain.setValueAtTime(0, t); env.gain.linearRampToValueAtTime(0.5 * vel * P.level, t + 0.35); mix.connect(lp); lp.connect(env); env.connect(rel); rel.connect(d);
      return { stop(t2) { A.release(rel, src, t2, 1.1); }, lp };
    },
    modal(d, t, f, vel, P) {
      const rel = A.gain(1), out = A.gain(P.level * 0.7 * vel), src = [];
      const ex = A.noiseSrc(), exg = A.gain(0), exl = A.filt('lowpass', P.tone, 0.7); exg.gain.setValueAtTime(1, t); exg.gain.setTargetAtTime(0, t + P.hard, P.hard * 0.7); ex.connect(exl); exl.connect(exg); ex.start(t, Math.random()); ex.stop(t + 1);
      const ratios = [1, 1 + 1.76 * P.structure, 1 + 4.4 * P.structure, 1 + 7.9 * P.structure, 1 + 12 * P.structure * 0.7];
      ratios.forEach((r, i) => { const bp = A.filt('bandpass', clamp(f * r * (1 + (Math.random() - 0.5) * P.drift * 0.02), 60, 15000), 25 + 120 * (1 - P.damp) / (1 + i * 0.4)); const g = A.gain((0.9 / (i * 0.8 + 1)) * (i ? P.bright : 1) * 6); exg.connect(bp); bp.connect(g); g.connect(out); });
      out.connect(rel); rel.connect(d);
      return { stop(t2) { A.release(rel, src, t2 + 0.6, 0.6); } };
    },
    choir(d, t, f, vel, P) {
      const F = [[800, 1150, 2900], [400, 1600, 2700], [350, 2000, 2800], [450, 800, 2830], [325, 700, 2530]];   // a e i o u
      const pos = clamp(P.vowel, 0, 1) * 4, i0 = Math.floor(pos), i1 = Math.min(4, i0 + 1), fr = pos - i0;
      const src = [], mix = A.gain(0.5), rel = A.gain(1), env = A.gain(0), out = A.gain(0);
      const o = A.osc('sawtooth', f, t), o2 = A.osc('sawtooth', f * 1.004, t); const sg = A.gain(0.5); o.connect(sg); o2.connect(sg); o.start(t); o2.start(t); src.push(o, o2);
      const v = A.osc('sine', 5.2, t), vg = A.gain(P.vib * 40); v.connect(vg); vg.connect(o.detune); vg.connect(o2.detune); v.start(t); src.push(v);
      if (P.breath > 0.01) { const n = A.noiseSrc(), ng = A.gain(P.breath * 0.5); n.connect(ng); ng.connect(sg); n.start(t); src.push(n); }
      const fl = [];
      [0, 1, 2].forEach((k) => { const bp = A.filt('bandpass', F[i0][k] + (F[i1][k] - F[i0][k]) * fr, 8 + k * 2); const g = A.gain([1, 0.55, 0.3][k] * 2.2); sg.connect(bp); bp.connect(g); g.connect(mix); fl.push(bp); });
      const lp = A.filt('lowpass', P.tone, 0.5); mix.connect(lp); lp.connect(env); env.connect(rel); rel.connect(d);
      env.gain.setValueAtTime(0, t); env.gain.linearRampToValueAtTime(0.6 * vel, t + 0.4);
      return { stop(t2) { A.release(rel, src, t2, 1.0); }, lp, fl, F, i0, i1, fr };
    },
    drone(d, t, f, vel, P) {
      const src = [], mix = A.gain(0.12), fold = A.shaper(P.fold, true), lp = A.filt('lowpass', P.cutoff, 0.5 + P.res * 12), env = A.gain(0), rel = A.gain(1);
      [-1.5, -0.5, 0.5, 1.5].forEach((k) => { const o = A.osc('sawtooth', f, t); o.detune.value = k * P.detune * 40; o.connect(mix); o.start(t); src.push(o); });
      const sub = A.osc('sine', f / 2, t), sg = A.gain(0.2); sub.connect(sg); sg.connect(mix); sub.start(t); src.push(sub);
      mix.connect(fold); fold.connect(lp); lp.connect(env); env.connect(rel); rel.connect(d);
      env.gain.setValueAtTime(0, t); env.gain.linearRampToValueAtTime(0.45 * vel, t + 1.2);
      const l = A.osc('sine', P.chaos, t), lg = A.gain(P.depth * 1800); l.connect(lg); lg.connect(lp.detune); l.start(t); src.push(l);
      if (P.dust > 0.02) { const n = A.noiseSrc(), hp = A.filt('highpass', 4000, 1), dg = A.gain(0), nl = A.osc('square', 6 + P.dust * 30, t), nlg = A.gain(P.dust * 0.12); nl.connect(nlg); nlg.connect(dg.gain); n.connect(hp); hp.connect(dg); dg.connect(d); n.start(t); nl.start(t); src.push(n, nl); }
      return { stop(t2) { A.release(rel, src, t2, 2); }, lp };
    },
    sub(d, t, f, vel, P) {
      const src = [], mix = A.gain(0.3), lp1 = A.filt('lowpass', P.cutoff, 0.5 + P.res * 10), lp2 = A.filt('lowpass', P.cutoff, 0.5), dist = A.shaper(P.dist * 0.6 + 1, false), env = A.gain(0), rel = A.gain(1);
      const o = A.osc('sawtooth', f, t), sub = A.osc('square', f / 2, t), sg = A.gain(P.sub); o.connect(mix); sub.connect(sg); sg.connect(mix); o.start(t); sub.start(t); src.push(o, sub);
      if (P.pw > 0.01) { const o2 = A.osc('square', f * 1.003, t), pg = A.gain(P.pw * 0.8); o2.connect(pg); pg.connect(mix); o2.start(t); src.push(o2); }
      const top = clamp(P.cutoff * 2 ** P.envamt, 40, 12000);
      [lp1, lp2].forEach((l) => { l.frequency.setValueAtTime(top, t); l.frequency.setTargetAtTime(P.cutoff, t, Math.max(0.01, P.fdecay / 3)); });
      if (P.wob > 0.02) { const l = A.osc('sine', P.rate, t), lg = A.gain(P.wob * 2200); l.connect(lg); lg.connect(lp1.detune); lg.connect(lp2.detune); l.start(t); src.push(l); }
      const pre = A.shaper(P.drive, false); mix.connect(pre); pre.connect(lp1); lp1.connect(lp2); lp2.connect(dist); dist.connect(env); env.connect(rel); rel.connect(d);
      env.gain.setValueAtTime(0, t); env.gain.linearRampToValueAtTime(0.5 * vel, t + 0.004);
      return { stop(t2) { A.release(rel, src, t2, 0.12); } };
    },
  };
  const FAMILY = { init: 'init', evolving_glass: 'glass', ghost_strings: 'strings', modal_garden: 'modal', hum_choir: 'choir', dust_drone: 'drone', sub_pressure: 'sub' };

  R.registerApp({
    id: 'atlas', name: 'Atlas', category: 'instrument',
    desc: 'Designed sounds on one surface: eight performance macros and a MORPH through each preset.',
    create: R.playApp({
      name: 'Atlas', pal: PAL, crumb: 'META-SYNTH',
      kit(inst) {
        inst.preset = 0;
        const apply = () => { if (!A.ctx) return; const P = effective(inst); inst.P = P; ensureFx(inst); inst.fx.set({ chorus: P.chorus, delay: P.delay, dtime: P.dtime, fb: P.fb, verb: P.verb }); inst.voices.forEach((v) => v && v.live && v.live(P)); };
        const m = (label) => ctl.f(label, 0, 1, 0, { fmt: fmt.pct, onChange: apply });
        const c = MACROS.map(m);
        c.push(ctl.f('MORPH', 0, 1, 0, { fmt: (v) => (v < 0.02 ? 'A' : v > 0.98 ? 'B' : `A>B ${Math.round(v * 100)}%`), onChange: apply }));
        c.push(ctl.int('Preset', 0, PRESETS.length - 1, 0, { fmt: (v) => PRESETS[v].name, onChange: (v) => { inst.preset = v; apply(); } }));
        c.push(ctl.f('Level', 0, 1, 0.8, { fmt: fmt.pct, onChange: (v) => inst.ch.setLevel(v) }));
        c.push(ctl.int('Voices', 1, 8, 6));
        inst.c = c;
        return { controls: c, hero: [[0, 1], [2, 6], [3, 4], [5, 7]], layers: ['PLAY', 'STATES'], browse: 8, routes: { sx: 7, sy: 1, hl: 2, hr: 3 } };
      },
      padLabel: (inst, p) => (inst.kit.layer.label === 'STATES' ? ['A', 'B'][R.rank(p)] || '' : R.noteName(midiOf(p))),
      padState: (inst, p) => (inst.kit.layer.label === 'STATES' ? ((R.rank(p) === 0 && inst.c[7].value < 0.5) || (R.rank(p) === 1 && inst.c[7].value >= 0.5) ? 3 : R.rank(p) < 2 ? 1 : 0) : midiOf(p) % 12 === 0 ? 1 : 0),
      line: () => 'Play',
      noteOn(inst, p, vel, t) {
        if (inst.kit.layer.label === 'STATES') { const rk = R.rank(p); if (rk < 2) { inst.c[7].base = rk; inst.c[7].onChange(rk); } return null; }
        ensureFx(inst); if (!inst.P) inst.P = effective(inst);
        if (inst.voices.size >= inst.c[10].value) { const first = inst.voices.keys().next().value; inst.voices.get(first).stop(A.T()); inst.voices.delete(first); }
        const fam = FAMILY[PRESETS[inst.preset].id], v = RECIPES[fam](inst.fx.in, t, hz(midiOf(p)), vel, inst.P);
        v.live = (P) => { if (v.lp) v.lp.frequency.setTargetAtTime(P.cutoff || P.tone || 2000, A.T(), 0.05); };
        return v;
      },
      enter(inst) { ensureFx(inst); inst.c[9].onChange(inst.c[9].base); },
      drawRight(fb, inst, pal) {
        const pr = PRESETS[inst.preset], ca = S.card(fb, pal, 'ATLAS / META-SYNTH');
        fb.ttf(pr.name, 326, 93, 16, pal.ink, { family: 'sans', weight: 600, maxW: 190 });
        fb.ttf(`${pr.category.toUpperCase()} ${inst.preset + 1}/${PRESETS.length}`, 612, 93, 12, pal.accent, { align: 'right', spacing: 1 });
        MACROS.forEach((n, i) => {
          const col = i % 2, row = Math.floor(i / 2), x = 326 + col * 147, y = 104 + row * 17, used = pr.macros[n].length > 0;
          fb.ttf(n[0] + n.slice(1).toLowerCase(), x, y + 11, 12, rgba(pal.ink, used ? 0.72 : 0.4), { spacing: 1 });
          S.meter(fb, pal, x + 66, y + 4, 70, 6, used ? inst.c[i].value : 0);
        });
        fb.ttf('MORPH', 326, 187, 12, pal.ink, { spacing: 1 });
        const mv = inst.c[7].value; fb.ttf(mv < 0.02 ? 'A' : mv > 0.98 ? 'B' : 'A>B', 612, 187, 12, pal.accent, { align: 'right', spacing: 1 });
        fb.rect(326, 197 + 7, 286, 2, rgba(pal.ink, 0.2));
        ['A', 'B'].forEach((l, s) => { const x = 326 + 270 * s; fb.rr(x, 197, 16, 16, 8, rgba(pal.ink, 0.12), rgba(pal.ink, 0.5)); fb.ttf(l, x + 8, 209, 12, pal.ink, { align: 'center' }); });
        fb.rr(326 + 280 * mv, 197, 6, 16, 3, pal.accent);
        S.scope(fb, pal, 326, 222, 286, 40); S.spectrum(fb, pal, 327, 223, 284, 38, 24, rgba(pal.accent, 0.85));
        fb.ttf(`${inst.voices.size} of ${inst.c[10].value} voices`, 326, 282, 12, rgba(pal.ink, 0.8)); fb.ttf(`cpu ${Math.round(2 + inst.voices.size * 1.5)}%`, 612, 282, 12, rgba(pal.ink, 0.8), { align: 'right' });
      },
    }),
  });

  function ensureFx(inst) { if (!inst.fx && A.ctx) { inst.fx = A.insertFx(inst.ch); if (inst.P) inst.fx.set({ chorus: inst.P.chorus, delay: inst.P.delay, dtime: inst.P.dtime, fb: inst.P.fb, verb: inst.P.verb }); } }
})();
