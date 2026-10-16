/* Tonestack: drive and a three-band tone stack with echo, hall, phaser, boost, scoop and fuzz on the pads. */
(() => {
  'use strict';
  const R = window.ROVE, A = R.audio, { ctl, fmt, rgba } = R, S = R.slint;
  const PAL = S.pal('#EEE6D3', '#3A2D21', '#995020', '#80715B');
  R.registerApp({
    id: 'tonestack', name: 'Tonestack', category: 'effect',
    desc: 'Drive and a bass, mid and treble stack, with echo, hall, phaser, boost, scoop and fuzz one pad away.',
    create: R.effectApp({
      name: 'Tonestack', pal: PAL, crumb: 'TONE STACK',
      kit(inst) {
        const db = (v) => `${v >= 0 ? '+' : ''}${v.toFixed(0)}`;
        const c = [ctl.f('Drive', 0, 1, 0.4, { fmt: fmt.pct }), ctl.f('Mid', -12, 12, 0, { fmt: db }), ctl.f('Bass', -12, 12, 0, { fmt: db }), ctl.f('Treble', -12, 12, 0, { fmt: db })];
        inst.c = c; inst.m = {}; inst.mode = 'Clean';
        const h = (label, key) => ({ label, control: 0, to: 0, hold: (d) => { inst.m[key] = d; inst.mode = d ? label[0] + label.slice(1).toLowerCase() : 'Clean'; } });
        return { controls: c, hero: [[0, 1], [2, 3]], throws: [h('BOOST', 'boost'), h('CLEAN', 'clean'), h('SCOOP', 'scoop'), h('FUZZ', 'fuzz'), h('ECHO', 'echo'), h('REPEAT', 'repeat'), h('HALL', 'hall'), h('PHASER', 'phaser')], routes: { sx: 0, sy: 1, hl: 2, hr: 3 } };
      },
      build(inst) {
        const c = A.ctx, inp = A.gain(1), out = A.gain(1), pre = A.shaper(2, false), low = A.filt('lowshelf', 220), mid = A.filt('peaking', 800, 0.9), high = A.filt('highshelf', 3200), post = A.gain(0.8);
        inp.connect(pre); pre.connect(low); low.connect(mid); mid.connect(high); high.connect(post); post.connect(out);
        const dl = c.createDelay(1), fbk = A.gain(0.35), em = A.gain(0), lpf = A.filt('lowpass', 3000); post.connect(dl); dl.connect(lpf); lpf.connect(fbk); fbk.connect(dl); lpf.connect(em); em.connect(out); dl.delayTime.value = 0.32;
        const ap = [A.filt('allpass', 600, 0.7), A.filt('allpass', 1400, 0.7)], pm = A.gain(0), lfo = A.osc('sine', 0.5, A.T()), lg = A.gain(500);
        post.connect(ap[0]); ap[0].connect(ap[1]); ap[1].connect(pm); pm.connect(out); lfo.connect(lg); lg.connect(ap[0].frequency); lg.connect(ap[1].frequency); lfo.start();
        inst.px = { pre, low, mid, high, post, fbk, em, pm, hall: 0 }; return { in: inp, out };
      },
      update(inst) {
        const c = inst.c, m = inst.m, p = inst.px, t = A.T(); let drive = c[0].value, mid = c[1].value;
        if (m.boost) drive = Math.min(1, drive + 0.25); if (m.clean) drive = 0; if (m.fuzz) drive = 1; if (m.scoop) mid -= 10;
        const key = Math.round(drive * 20) + (m.fuzz ? 100 : 0); if (p.key !== key) { p.key = key; p.pre.curve = A.shaper(1 + drive * (m.fuzz ? 30 : 8), false).curve; }
        p.low.gain.setTargetAtTime(c[2].value, t, 0.03); p.mid.gain.setTargetAtTime(mid, t, 0.03); p.high.gain.setTargetAtTime(c[3].value, t, 0.03); p.post.gain.setTargetAtTime((m.boost ? 1.2 : 0.8) / (1 + drive * 0.6), t, 0.03);
        p.em.gain.setTargetAtTime(m.echo || m.repeat ? 0.6 : 0, t, 0.03); p.fbk.gain.setTargetAtTime(m.repeat ? 0.72 : 0.35, t, 0.03); p.pm.gain.setTargetAtTime(m.phaser ? 0.8 : 0, t, 0.03); inst.ch.setSend(m.hall ? 0.8 : 0.08);
        inst.gated = !!m.boost || inst.mode !== 'Clean';
      },
      drawRight(fb, inst, pal) {
        S.card(fb, pal, 'TONE / SIGNAL CHAIN');
        fb.ttf(inst.mode, 326, 90, 12, pal.accent, { family: 'sans', weight: 700 }); fb.disc(552, 82, 9, '#E5484D'); fb.ttf('GATED', 612, 92, 12, pal.accent, { align: 'right', spacing: 1 });
        S.scope(fb, pal, 326, 102, 286, 82); S.wave(fb, pal, 326, 102, 286, 82, pal.accent);
        fb.ttf('OUTPUT', 326, 210, 12, rgba(pal.ink, 0.72), { spacing: 1 }); S.meter(fb, pal, 326, 288, 286, 6, inst.level * 3, pal.accent);
      },
    }),
  });
})();
