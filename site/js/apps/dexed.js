/* Dexed: a six-operator FM synth that plays real DX7 voices. The 32 voices are unpacked from the
   SysEx bank in dx7_presets/, and the operator routing is the DX7's own 32-algorithm table from the
   msfa engine (vendor/dexed/msfa/fm_core.cc). The operators are Web Audio oscillators; envelope
   speeds use the msfa rate law, but key scaling, pitch EG and LFO shaping are not modelled. */
(() => {
  'use strict';
  const R = window.ROVE, A = R.audio, { ctl, fmt } = R, S = R.slint;
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const PAL = S.pal('#101B19', '#E5F0EF', '#9CC8F0', '#7E93A1');
  const VOICES = R.data.dx7;
  const ALG = [
    [0xc1, 0x11, 0x11, 0x14, 0x01, 0x14], [0x01, 0x11, 0x11, 0x14, 0xc1, 0x14], [0xc1, 0x11, 0x14, 0x01, 0x11, 0x14], [0xc1, 0x11, 0x94, 0x01, 0x11, 0x14],
    [0xc1, 0x14, 0x01, 0x14, 0x01, 0x14], [0xc1, 0x94, 0x01, 0x14, 0x01, 0x14], [0xc1, 0x11, 0x05, 0x14, 0x01, 0x14], [0x01, 0x11, 0xc5, 0x14, 0x01, 0x14],
    [0x01, 0x11, 0x05, 0x14, 0xc1, 0x14], [0x01, 0x05, 0x14, 0xc1, 0x11, 0x14], [0xc1, 0x05, 0x14, 0x01, 0x11, 0x14], [0x01, 0x05, 0x05, 0x14, 0xc1, 0x14],
    [0xc1, 0x05, 0x05, 0x14, 0x01, 0x14], [0xc1, 0x05, 0x11, 0x14, 0x01, 0x14], [0x01, 0x05, 0x11, 0x14, 0xc1, 0x14], [0xc1, 0x11, 0x02, 0x25, 0x05, 0x14],
    [0x01, 0x11, 0x02, 0x25, 0xc5, 0x14], [0x01, 0x11, 0x11, 0xc5, 0x05, 0x14], [0xc1, 0x14, 0x14, 0x01, 0x11, 0x14], [0x01, 0x05, 0x14, 0xc1, 0x14, 0x14],
    [0x01, 0x14, 0x14, 0xc1, 0x14, 0x14], [0xc1, 0x14, 0x14, 0x14, 0x01, 0x14], [0xc1, 0x14, 0x14, 0x01, 0x14, 0x04], [0xc1, 0x14, 0x14, 0x14, 0x04, 0x04],
    [0xc1, 0x14, 0x14, 0x04, 0x04, 0x04], [0xc1, 0x05, 0x14, 0x01, 0x14, 0x04], [0x01, 0x05, 0x14, 0xc1, 0x14, 0x04], [0x04, 0xc1, 0x11, 0x14, 0x01, 0x14],
    [0xc1, 0x14, 0x01, 0x14, 0x04, 0x04], [0x04, 0xc1, 0x11, 0x14, 0x04, 0x04], [0xc1, 0x14, 0x04, 0x04, 0x04, 0x04], [0xc4, 0x04, 0x04, 0x04, 0x04, 0x04],
  ];
  const LEVEL_LUT = [0, 5, 9, 13, 17, 20, 23, 25, 27, 29, 31, 33, 35, 37, 39, 41, 42, 43, 45, 46];
  const sl = (l) => (l >= 20 ? 28 + l : LEVEL_LUT[l]);               // msfa scaleoutlevel
  const dbOf = (L, OL) => (sl(L) + sl(OL) - 254) * 0.75;              // 0 dB at level 99 / output 99
  const dbPerSec = (rate) => { const qr = (rate * 41) >> 6; return (4 + (qr & 3)) * 2 ** ((qr >> 2) - 13) * 0.75 * 750; };
  const BETA = 4;                                                       // modulation index at full output level

  function voice(inst, t, midi, vel) {
    const c = A.ctx, v = VOICES[inst.c[0].value], f0 = A.midiHz(midi + (v.trn - 24)), alg = ALG[v.alg], dest = inst.out;
    const buses = [[], [], []];                                          // bus 0 is the output; 1 and 2 carry modulation
    const nodes = [], gains = [], carriers = [];
    const nCar = alg.filter((fl) => (fl & 7) === 4 || (fl & 3) === 0).length || 1;
    const vib = inst.vibrato;
    for (let i = 0; i < 6; i++) {                                        // i = 0 is OP6
      const fl = alg[i], op = v.ops[5 - i], ol = op.ol;
      const ratio = op.fixed ? 10 ** (op.coarse % 4) : (op.coarse === 0 ? 0.5 : op.coarse) * (1 + op.fine / 100);
      const fr = op.fixed ? ratio * (1 + op.fine / 100 * 9) : f0 * ratio, det = (op.det - 7) * 1.4;
      const o = A.osc('sine', clamp(fr, 1, 12000), t); o.detune.value = det; o.start(t); o.stop(t + 60); nodes.push(o);
      const bend = inst.bendNode; if (bend) bend.connect(o.detune); if (vib) vib.connect(o.detune);
      const env = A.gain(0), vdb = -(1 - vel) * op.kvs * 6;
      // envelope: start at L4, run R1->L1, R2->L2, R3->L3, hold; key-off runs R4->L4
      const amp = (L) => (L === 0 ? 1e-5 : Math.max(1e-5, 10 ** ((dbOf(L, ol) + vdb) / 20)));
      let cur = amp(op.l[3]); env.gain.setValueAtTime(cur, t); let tt = t;
      for (let s = 0; s < 3; s++) { const tgt = amp(op.l[s]), dB = Math.abs(20 * Math.log10(tgt / cur)); tt += Math.max(0.002, dB / dbPerSec(op.r[s])); env.gain.exponentialRampToValueAtTime(tgt, Math.min(tt, t + 60)); cur = tgt; }
      o.connect(env); gains.push({ env, op, amp, ol, vdb });
      const fbOn = (fl & 0x40) && v.fb > 0;
      let out = env;
      if (fl & 0x30) {                                                   // takes modulation from bus 1 or 2
        const bus = buses[(fl & 0x10) ? 1 : 2];
        bus.forEach((src) => { src.connect(o.frequency); });
      }
      if (fbOn) { const dl = c.createDelay(0.01); dl.delayTime.value = 128 / c.sampleRate; const fg = A.gain(fr * (v.fb / 7) * 2); env.connect(dl); dl.connect(fg); fg.connect(o.frequency); }
      const target = fl & 3, add = !!(fl & 4);
      if (target === 0) { const og = A.gain(0.32 / Math.sqrt(nCar)); env.connect(og); og.connect(dest); carriers.push(og); }
      else {
        const mg = A.gain(BETA * fr);                                    // PM index -> frequency deviation in Hz
        env.connect(mg); if (!add) buses[target] = []; buses[target].push(mg);
      }
    }
    return {
      stop(t2) {
        gains.forEach(({ env, op, amp }) => {
          if (env.gain.cancelAndHoldAtTime) env.gain.cancelAndHoldAtTime(t2); else { env.gain.cancelScheduledValues(t2); }
          const cur = amp(op.l[2]), tgt = amp(op.l[3]), dB = Math.abs(20 * Math.log10(tgt / cur));
          const dur = clamp(dB / dbPerSec(op.r[3]), 0.01, 8);
          env.gain.exponentialRampToValueAtTime(tgt, t2 + dur);
        });
        const longest = Math.min(9, Math.max(...gains.map(({ op, amp }) => Math.abs(20 * Math.log10(amp(op.l[3]) / amp(op.l[2]))) / dbPerSec(op.r[3]))) + 0.2);
        nodes.forEach((o) => { try { o.stop(t2 + Math.max(0.1, longest)); } catch (_) { /* ok */ } });
        carriers.forEach((g) => g.gain.setTargetAtTime(0, t2 + Math.max(0.1, longest) - 0.05, 0.02));
      },
    };
  }

  R.registerApp({
    id: 'dexed', name: 'Dexed', category: 'instrument',
    desc: 'A six-operator FM synth that plays real DX7 voices from SysEx banks, with the DX7’s 32 algorithms.',
    create: R.playApp({
      name: 'Dexed', pal: PAL, crumb: 'FM SYNTH',
      kit(inst) {
        const c = [
          ctl.int('Voice', 0, 31, 1, { fmt: (v) => `${VOICES[v].name.trim()} ${v + 1}/32` }),
          ctl.f('Level', 0, 1, 0.8, { fmt: fmt.pct, onChange: (v) => inst.ch.setLevel(v * 1.1) }),
          ctl.f('Bend', -2, 2, 0, { fmt: fmt.st, onChange: () => bend(inst) }),
          ctl.f('Mod', 0, 1, 0, { fmt: fmt.pct, onChange: () => bend(inst) }),
        ];
        inst.c = c;
        return { controls: c, hero: [[0, 1], [2, 3]], layers: ['KEYS'], browse: 0, routes: { sx: 2, sy: 3 } };
      },
      padLabel: (inst, p) => R.noteName(R.padNote(p, { base: 48 })),
      padState: (inst, p) => (R.padNote(p, { base: 48 }) % 12 === 0 ? 1 : 0),
      noteOn(inst, p, vel, t) { ensure(inst); if (inst.voices.size >= 8) { const k = inst.voices.keys().next().value; inst.voices.get(k).stop(A.T()); inst.voices.delete(k); } return voice(inst, t, R.padNote(p, { base: 48 }), vel); },
      enter(inst) { ensure(inst); },
      drawRight(fb, inst, pal) { S.paramCard(fb, pal, 'VOICE / CONTROL', 'Voice', `${VOICES[inst.c[0].base].name.trim()} ${inst.c[0].base + 1}/32`); },
      line: (inst) => (inst.voices.size ? `${inst.voices.size} voices` : 'Play'),
    }),
  });

  function ensure(inst) {
    if (inst.out || !A.ctx) return;
    inst.out = A.gain(1); inst.out.connect(inst.ch.input);
    inst.bendNode = A.gain(0); const dc = A.ctx.createConstantSource(); dc.offset.value = 1; dc.start(); dc.connect(inst.bendNode);
    inst.vibrato = A.gain(0); const l = A.osc('sine', 5.5, A.T()); l.connect(inst.vibrato); l.start();
    bend(inst);
  }
  function bend(inst) { if (!inst.bendNode) return; inst.bendNode.gain.setTargetAtTime(inst.c[2].value * 100, A.T(), 0.02); inst.vibrato.gain.setTargetAtTime(inst.c[3].value * 40, A.T(), 0.02); }
})();
