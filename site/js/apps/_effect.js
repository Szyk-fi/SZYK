/* Shared scaffolding for effect apps. In the real device an effect's Source row picks any app's audio;
   in this preview the source is the built-in demo loop, which plays while an effect is open. F3 bypasses. */
(() => {
  'use strict';
  const R = window.ROVE, A = R.audio;

  /* spec: { name, pal, crumb, kit(inst) -> { controls, hero, throws, browse?, routes? }, build(inst) -> { in, out },
             update(inst), drawRight(fb, inst, pal), line?(inst) }  */
  R.effectApp = (spec) => R.playApp({
    effect: spec,
    name: spec.name, title: spec.title, colW: spec.colW, pal: spec.pal, crumb: spec.crumb || 'EFFECT',
    kit(inst) {
      inst.bypass = false; inst.hold = {};
      const cfg = spec.kit(inst); cfg.native = false;
      return cfg;
    },
    padLabel: () => '', padState: () => 0,
    line: (inst) => (spec.line ? spec.line(inst) : 'Play'),
    noteOn() { return null; },
    enter(inst) { build(inst); A.demoLoop.start(); inst.on = true; },
    leave(inst) { A.demoLoop.stop(); A.demoBus.disconnect(); inst.chain = null; inst.on = false; },
    transport: (inst) => ({ label: () => 'BYPASS', running: () => inst.bypass, toggle: () => { inst.bypass = !inst.bypass; route(inst); } }),
    tick(inst) { if (inst.chain && spec.update) spec.update(inst); inst.level = inst.ch.rms(); },
    drawRight: spec.drawRight,
  });

  function build(inst) {
    if (!A.ctx) return;
    const c = A.ctx, spec = inst.spec;
    if (inst.src) { try { inst.src.disconnect(); } catch (_) { /* ok */ } }
    inst.src = A.gain(1); A.demoBus.connect(inst.src);
    inst.inMeter = c.createAnalyser(); inst.inMeter.fftSize = 512; inst.src.connect(inst.inMeter); inst._ib = new Float32Array(512);
    inst.dry = A.gain(0); inst.wetOut = A.gain(1); inst.src.connect(inst.dry); inst.dry.connect(inst.ch.input);
    inst.chain = inst.spec.effect.build(inst); inst.src.connect(inst.chain.in); inst.chain.out.connect(inst.wetOut); inst.wetOut.connect(inst.ch.input);
    route(inst);
  }
  function route(inst) { if (!inst.dry) return; inst.dry.gain.setTargetAtTime(inst.bypass ? 1 : 0, A.T(), 0.02); inst.wetOut.gain.setTargetAtTime(inst.bypass ? 0 : 1, A.T(), 0.02); }
  R.inLevel = (inst) => { if (!inst.inMeter) return 0; inst.inMeter.getFloatTimeDomainData(inst._ib); let s = 0; for (let i = 0; i < 512; i++) s += inst._ib[i] ** 2; return Math.sqrt(s / 512); };
})();
