/* Sequencer: a drum step sequencer, eight tracks of sixteen steps across eight patterns. The pads edit the
   steps of the selected track, D-pad up/down picks the track, F3 plays. */
(() => {
  'use strict';
  const R = window.ROVE, A = R.audio, { ctl, fmt, rgba } = R, S = R.slint;
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const PAL = S.pal('#080408', '#FFE3DE', '#FF393A', '#7B5152');
  const TRACKS = [['Kick', 'kick'], ['Snare', 'snare'], ['Hat', 'chat'], ['Clap', 'clap'], ['Open hat', 'ohat'], ['Tom', 'tomM'], ['Rim', 'rim'], ['Bass', 'bass']];
  const DEMO = { 0: [0, 4, 8, 12], 1: [4, 12], 2: [0, 2, 4, 6, 8, 10, 12, 14], 3: [12], 4: [6, 14], 7: [0, 3, 6, 10] };

  R.registerApp({
    id: 'sequencer', name: 'Sequencer', category: 'sequencer',
    desc: 'A drum step sequencer: eight tracks of sixteen steps, eight patterns, swing and decay.',
    create: R.playApp({
      name: 'Sequencer', pal: PAL, crumb: 'STEP SEQUENCER',
      kit(inst) {
        const c = [
          ctl.f('Tempo', 60, 200, 120, { fmt: (v) => `${Math.round(v)}` }), ctl.f('Swing', 0, 0.5, 0, { fmt: fmt.d2 }), ctl.f('Level', 0, 1, 0.8, { fmt: fmt.d2, onChange: (v) => inst.ch.setLevel(v * 1.1) }), ctl.f('Decay', 0, 1, 0.5, { fmt: fmt.d2 }),
          ctl.int('Track', 0, 7, 0, { fmt: (v) => `T${v + 1} ${TRACKS[v][0]}` }), ctl.int('Pattern', 0, 7, 0, { fmt: (v) => String(v + 1) }),
        ];
        inst.c = c; inst.run = false; inst.step = -1;
        inst.pat = Array.from({ length: 8 }, (_, p) => Array.from({ length: 8 }, (_, t) => { const g = Array(16).fill(0); if (p === 0 && DEMO[t]) DEMO[t].forEach((s) => { g[s] = 1; }); return g; }));
        inst.flash = Array(8).fill(0);
        return { controls: c, hero: [[0, 1], [2, 3]], layers: ['STEP'], browse: 4, routes: { sx: 0, sy: 3 } };
      },
      padLabel: (inst, p) => String(p + 1),
      padState: (inst, p) => (inst.pat[inst.c[5].value][inst.c[4].value][p] ? 3 : 0),
      line: (inst) => `› T${inst.c[4].value + 1} ${TRACKS[inst.c[4].value][0]}`,
      enter(inst) { ensure(inst); },
      noteOn(inst, p) { const g = inst.pat[inst.c[5].value][inst.c[4].value]; g[p] = g[p] ? 0 : 1; inst.kit.flash(`T${inst.c[4].value + 1} step ${p + 1} ${g[p] ? 'on' : 'off'}`); if (A.ctx && g[p]) trig(inst, inst.c[4].value, A.T() + 0.002, 0.8); return null; },
      transport: (inst) => ({ label: () => (inst.run ? 'STOP' : 'PLAY'), running: () => inst.run, toggle: () => toggle(inst) }),
      tick(inst) { inst.flash = inst.flash.map((f) => f * 0.9); },
      drawRight(fb, inst, pal) {
        S.card(fb, pal, 'PATTERN / PERFORMANCE');
        for (let i = 0; i < 8; i++) { const x = 326 + i * 36, on = i === inst.c[5].value; fb.rr(x, 80, 32, 14, 2, on ? pal.accent : rgba(pal.ink, 0.1)); fb.ttf(String(i + 1), x + 16, 91, 12, on ? pal.paper : rgba(pal.ink, 0.8), { align: 'center' }); }
        fb.ttf(`Track ${inst.c[4].value + 1} — grid edits this`, 326, 112, 12, pal.accent);
        const g = inst.pat[inst.c[5].value][inst.c[4].value], w = 64, h = 38;
        for (let i = 0; i < 16; i++) {
          const x = 326 + (i % 4) * (w + 8), y = 120 + Math.floor(i / 4) * (h + 6), now = inst.run && inst.step % 16 === i;
          fb.rr(x, y, w, h, 3, g[i] ? rgba(pal.accent, 0.35) : rgba(pal.ink, 0.04), now ? pal.ink : g[i] ? pal.accent : rgba(pal.ink, 0.2), now ? 2 : 1);
          fb.ttf(String(i + 1), x + w - 5, y + 13, 11, rgba(pal.ink, 0.7), { align: 'right' });
        }
        TRACKS.forEach((t, i) => { fb.disc(326 + i * 36, 296, 7, inst.flash[i] > 0.1 ? pal.accent : rgba(pal.ink, i === inst.c[4].value ? 0.5 : 0.15)); });
      },
    }),
  });

  function ensure(inst) { if (!inst.bus && A.ctx) { inst.bus = A.gain(1); inst.bus.connect(inst.ch.input); inst.ch.setSend(0.1); } }
  function trig(inst, t, time, vel) {
    ensure(inst); const dm = 0.5 + inst.c[3].value * 1.2;
    if (t === 7) { const v = A.subVoice(inst.bus, time, A.midiHz(33 + [0, 0, 3, 5][inst.step % 4 | 0]), vel, { wave: 'sawtooth', sub: 0.8, cutoff: 500, envAmt: 2, res: 4, dec: 0.15 * dm, sus: 0.3, rel: 0.1, level: 0.5 }); v.stop(time + 0.22 * dm); }
    else A.drum(TRACKS[t][1], inst.bus, time, vel, 1, dm);
    inst.flash[t] = 1;
  }
  function toggle(inst) {
    if (!A.ctx) return; ensure(inst);
    if (inst.run) { inst.sched.stop(); inst.run = false; inst.ch.active = false; return; }
    inst.run = true; inst.ch.active = true;
    inst.sched = new A.Scheduler(() => 60 / inst.c[0].value / 4, (s, t) => {
      const st = s % 16, sw = st % 2 ? inst.c[1].value * (60 / inst.c[0].value / 4) : 0; inst.step = s;
      for (let tr = 0; tr < 8; tr++) if (inst.pat[inst.c[5].value][tr][st]) trig(inst, tr, t + sw, 0.85);
    });
    inst.sched.start();
  }
})();
