/* Web Audio engine for the browser preview.
   Not the native engine: these are compact voices written for this page. Every app talks to
   the same pieces the real device shares between apps: a mixer channel each, one space (echo +
   room) return, and a demo loop that effect apps can take as their source. */
(() => {
  'use strict';
  const R = (window.ROVE = window.ROVE || {});
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const midiHz = (m) => 440 * 2 ** ((m - 69) / 12);

  const A = {
    ctx: null, volume: 0.75, channels: new Map(), onReady: [],
    T() { return A.ctx ? A.ctx.currentTime : 0; },
  };
  const volToGain = (v) => v * v * 0.9;

  A.init = function init() {
    if (A.ctx) { if (A.ctx.state === 'suspended') A.ctx.resume(); return true; }
    const AC = window.AudioContext || window.webkitAudioContext;
    if (!AC) return false;
    const c = (A.ctx = new AC({ latencyHint: 'interactive' }));
    A.mix = c.createGain();                                   // every channel sums here
    A.master = c.createGain(); A.master.gain.value = volToGain(A.volume);
    const comp = c.createDynamicsCompressor();
    comp.threshold.value = -12; comp.knee.value = 10; comp.ratio.value = 5; comp.attack.value = 0.004; comp.release.value = 0.2;
    A.analyser = c.createAnalyser(); A.analyser.fftSize = 2048; A.analyser.smoothingTimeConstant = 0.5;
    A.mix.connect(A.master); A.master.connect(comp); comp.connect(A.analyser); A.analyser.connect(c.destination);

    // shared space: ping-pong-ish echo into a room, returned to the mix
    A.space = c.createGain();
    const dl = c.createDelay(1.5); dl.delayTime.value = 0.28;
    const fb = c.createGain(); fb.gain.value = 0.34;
    const lp = c.createBiquadFilter(); lp.type = 'lowpass'; lp.frequency.value = 3400;
    A.space.connect(dl); dl.connect(lp); lp.connect(fb); fb.connect(dl);
    const dOut = c.createGain(); dOut.gain.value = 0.5; dl.connect(dOut); dOut.connect(A.mix);
    const verb = c.createConvolver();
    const len = Math.floor(c.sampleRate * 2.2), ir = c.createBuffer(2, len, c.sampleRate);
    for (let ch = 0; ch < 2; ch++) { const d = ir.getChannelData(ch); for (let i = 0; i < len; i++) d[i] = (Math.random() * 2 - 1) * (1 - i / len) ** 2.8; }
    verb.buffer = ir;
    const vOut = c.createGain(); vOut.gain.value = 0.55;
    A.space.connect(verb); dl.connect(verb); verb.connect(vOut); vOut.connect(A.mix);

    const nl = c.sampleRate * 2;
    A.noise = c.createBuffer(1, nl, c.sampleRate);
    const nd = A.noise.getChannelData(0);
    for (let i = 0; i < nl; i++) nd[i] = Math.random() * 2 - 1;

    // the demo loop's own bus: not heard by itself, only through an effect app
    A.demoBus = c.createGain();
    A.channels.forEach((ch) => ch._wire());
    A.onReady.splice(0).forEach((f) => f());
    return true;
  };

  /* a mixer channel per app: input -> fader -> meter -> mix, plus a send to the shared space */
  A.channel = function channel(name) {
    if (A.channels.has(name)) return A.channels.get(name);
    const ch = {
      name, level: 1, send: 0.12, muted: false, input: null, fader: null, meter: null, sendGain: null, _buf: null,
      _wire() {
        if (!A.ctx || this.input) return;
        const c = A.ctx;
        this.input = c.createGain();
        this.fader = c.createGain(); this.fader.gain.value = this.muted ? 0 : this.level;
        this.meter = c.createAnalyser(); this.meter.fftSize = 512; this._buf = new Float32Array(512);
        this.sendGain = c.createGain(); this.sendGain.gain.value = this.send;
        this.input.connect(this.fader); this.fader.connect(this.meter); this.fader.connect(A.mix);
        this.fader.connect(this.sendGain); this.sendGain.connect(A.space);
      },
      setLevel(v) { this.level = clamp(v, 0, 1.5); if (this.fader) this.fader.gain.setTargetAtTime(this.muted ? 0 : this.level, A.T(), 0.02); },
      setSend(v) { this.send = clamp(v, 0, 1); if (this.sendGain) this.sendGain.gain.setTargetAtTime(this.send, A.T(), 0.03); },
      rms() {
        if (!this.meter) return 0;
        this.meter.getFloatTimeDomainData(this._buf);
        let s = 0; for (let i = 0; i < 512; i++) s += this._buf[i] * this._buf[i];
        return Math.sqrt(s / 512);
      },
      active: false,    // set by the app while it is making sound or running
    };
    A.channels.set(name, ch);
    if (A.ctx) ch._wire();
    return ch;
  };
  A.setVolume = (v) => { A.volume = v; if (A.ctx) A.master.gain.setTargetAtTime(volToGain(v), A.T(), 0.02); };

  /* ------------------------------------------------------------------ small helpers */
  const osc = (type, f, t) => { const o = A.ctx.createOscillator(); o.type = type; o.frequency.setValueAtTime(f, t); return o; };
  const gain = (v) => { const g = A.ctx.createGain(); g.gain.value = v; return g; };
  const filt = (type, f, q) => { const b = A.ctx.createBiquadFilter(); b.type = type; b.frequency.value = f; if (q != null) b.Q.value = q; return b; };
  const noiseSrc = () => { const s = A.ctx.createBufferSource(); s.buffer = A.noise; s.loop = true; return s; };
  const expTo = (p, v, t) => p.exponentialRampToValueAtTime(Math.max(v, 1e-4), t);
  const stopAll = (nodes, t) => nodes.forEach((n) => { try { n.stop(t); } catch (_) { /* already stopped */ } });
  /* A voice's last stage is a separate `rel` gain, so a release never fights the note's own envelope. */
  function release(rel, sources, t, time) {
    rel.gain.cancelScheduledValues(t);
    rel.gain.setValueAtTime(1, t);
    rel.gain.setTargetAtTime(0, t, Math.max(0.005, time / 4));
    stopAll(sources, t + time * 1.8 + 0.05);
  }
  Object.assign(A, { osc, gain, filt, noiseSrc, expTo, release, stopAll, midiHz, clamp });

  /* ------------------------------------------------------------------ voices */
  /* Subtractive voice. p: wave, wave2, mix2, detune (cents), sub, noise, pw,
     cutoff (Hz), res (Q), envAmt (octaves), keyTrack (0..1), atk, dec, sus, rel, level */
  A.subVoice = function subVoice(dest, t, f, vel, p) {
    const sources = [];
    const mixG = gain(1);
    const addOsc = (type, freq, lvl, det) => {
      const o = osc(type, freq, t); if (det) o.detune.value = det;
      const g = gain(lvl); o.connect(g); g.connect(mixG); o.start(t); sources.push(o); return o;
    };
    const norm = 1 / (1 + (p.mix2 || 0) + (p.sub || 0));
    addOsc(p.wave || 'sawtooth', f, norm, -(p.detune || 0) / 2);
    if (p.mix2) addOsc(p.wave2 || p.wave || 'sawtooth', f, norm * p.mix2, (p.detune || 0) / 2);
    if (p.sub) addOsc('square', f / 2, norm * p.sub);
    if (p.noise) { const n = noiseSrc(); const g = gain(p.noise * 0.5); n.connect(g); g.connect(mixG); n.start(t); sources.push(n); }
    const lp = filt(p.filter || 'lowpass', 1000, p.res || 0.7);
    const base = clamp((p.cutoff || 2000) * (1 + (p.keyTrack || 0) * (f / 261.6 - 1)), 40, 18000);
    const top = clamp(base * 2 ** (p.envAmt || 0), 40, 18000);
    lp.frequency.setValueAtTime(top, t);
    lp.frequency.setTargetAtTime(base, t + 0.005, Math.max(0.02, (p.dec || 0.3) / 3));
    const env = gain(0), atk = Math.max(0.002, p.atk || 0.005);
    const peak = (p.level || 0.5) * vel, sus = clamp(p.sus == null ? 0.8 : p.sus, 0, 1) * peak;
    env.gain.setValueAtTime(0, t);
    env.gain.linearRampToValueAtTime(peak, t + atk);
    env.gain.setTargetAtTime(sus, t + atk, Math.max(0.02, (p.dec || 0.3) / 3));
    const rel = gain(1);
    mixG.connect(lp); lp.connect(env); env.connect(rel); rel.connect(dest);
    sources.forEach((s) => { try { s.stop(t + 40); } catch (_) { /* ignore */ } });
    return {
      filter: lp, env, sources,
      stop(t2) { release(rel, sources, t2, Math.max(0.03, p.rel == null ? 0.25 : p.rel)); },
    };
  };

  /* Three-operator FM stack (op3 -> op2 -> carrier, plus a parallel op1->carrier), DX-style. */
  A.fmVoice = function fmVoice(dest, t, f, vel, p) {
    const car = osc('sine', f * (p.rc || 1), t);
    const m1 = osc('sine', f * (p.r1 || 1), t), m2 = osc('sine', f * (p.r2 || 2), t);
    const g1 = gain(0), g2 = gain(0);
    const d = Math.max(0.05, p.decay || 1.2);
    g1.gain.setValueAtTime(f * (p.i1 || 1.5), t); expTo(g1.gain, f * (p.i1 || 1.5) * (p.sustainIdx == null ? 0.2 : p.sustainIdx) + 0.01, t + d * 0.7);
    g2.gain.setValueAtTime(f * (p.i2 || 0.5), t); expTo(g2.gain, 0.01, t + d * 0.3);
    m1.connect(g1); g1.connect(car.frequency); m2.connect(g2); g2.connect(car.frequency);
    const env = gain(0), atk = Math.max(0.002, p.atk || 0.004);
    env.gain.setValueAtTime(0, t); env.gain.linearRampToValueAtTime((p.level || 0.5) * vel, t + atk);
    if (p.sustain) env.gain.setTargetAtTime((p.level || 0.5) * vel * p.sustain, t + atk, d / 3);
    else expTo(env.gain, 0.0004, t + atk + d);
    const rel = gain(1);
    car.connect(env); env.connect(rel); rel.connect(dest);
    const src = [car, m1, m2];
    src.forEach((o) => { o.start(t); o.stop(t + atk + d + 40); });
    return { stop(t2) { if (p.sustain) release(rel, src, t2, p.rel || 0.3); else release(rel, src, t2, Math.min(0.5, d * 0.5)); } };
  };

  /* Karplus-Strong string. Buffer is filled at note-on; playbackRate trims the integer loop to exact pitch. */
  A.pluckVoice = function pluckVoice(dest, t, f, vel, p) {
    const sr = A.ctx.sampleRate, N = Math.max(2, Math.round(sr / f));
    const b = clamp(p.damp == null ? 0.25 : p.damp, 0.02, 0.5), t60 = clamp(p.t60 || 2, 0.2, 6);
    const g = Math.pow(0.001, (N + b) / (sr * t60));
    const len = Math.floor(sr * Math.min(5, t60 * 0.8 + 0.4));
    const buf = A.ctx.createBuffer(1, len, sr), d = buf.getChannelData(0);
    let prev = 0, mean = 0, pk = 0;
    for (let i = 0; i < N; i++) { prev = prev * (p.soft == null ? 0.3 : p.soft) + (Math.random() * 2 - 1) * (1 - (p.soft == null ? 0.3 : p.soft)); d[i] = prev; mean += prev; }
    mean /= N;
    for (let i = 0; i < N; i++) { d[i] -= mean; pk = Math.max(pk, Math.abs(d[i])); }
    for (let i = 0; i < N; i++) d[i] *= 0.75 / (pk || 1);
    for (let i = N; i < len; i++) d[i] = g * ((1 - b) * d[i - N] + b * (i - N - 1 >= 0 ? d[i - N - 1] : 0));
    const src = A.ctx.createBufferSource(); src.buffer = buf;
    src.playbackRate.value = (sr / (N + b)) / f;
    const env = gain((p.level || 1) * vel), rel = gain(1);
    src.connect(env); env.connect(rel); rel.connect(dest); src.start(t);
    return { stop(t2) { if (p.sustains) release(rel, [src], t2, 0.4); } };
  };

  /* ------------------------------------------------------------------ drums */
  const METAL = [205.3, 304.4, 369.6, 522.7, 540, 800];
  function dTone(dest, type, f0, f1, sweep, t, peak, decay) {
    const o = osc(type, f0, t); if (f1 !== f0) expTo(o.frequency, f1, t + sweep);
    const g = gain(0); g.gain.setValueAtTime(peak, t); expTo(g.gain, 0.0003, t + decay);
    o.connect(g); g.connect(dest); o.start(t); o.stop(t + decay + 0.05); return g;
  }
  function dNoise(dest, type, f, q, t, peak, decay, f2) {
    const s = noiseSrc(), fl = filt(type, f, q);
    if (f2) { fl.frequency.setValueAtTime(f, t); expTo(fl.frequency, f2, t + decay); }
    const g = gain(0); g.gain.setValueAtTime(peak, t); expTo(g.gain, 0.0003, t + decay);
    s.connect(fl); fl.connect(g); g.connect(dest); s.start(t, Math.random()); s.stop(t + decay + 0.05); return g;
  }
  function dMetal(dest, t, peak, decay, tune, hp, bp) {
    const g = gain(0); g.gain.setValueAtTime(peak, t); expTo(g.gain, 0.0003, t + decay);
    const b = filt('bandpass', bp, 0.7), h = filt('highpass', hp, 0.8);
    b.connect(h); h.connect(g); g.connect(dest);
    METAL.forEach((f) => { const o = osc('square', f * tune * 2, t); const m = gain(0.7); o.connect(m); m.connect(b); o.start(t); o.stop(t + decay + 0.05); });
    return g;
  }
  const open = new WeakMap();
  const DRUM = {
    kick: (d, t, v, tu, dm) => { dTone(d, 'sine', 160 * tu, 45 * tu, 0.11, t, 1.0 * v, 0.55 * dm); dNoise(d, 'lowpass', 3000, 0.5, t, 0.2 * v, 0.012); },
    kick2: (d, t, v, tu, dm) => { dTone(d, 'sine', 240 * tu, 55 * tu, 0.07, t, 1.0 * v, 0.4 * dm); dTone(d, 'triangle', 90 * tu, 50 * tu, 0.1, t, 0.4 * v, 0.2 * dm); dNoise(d, 'highpass', 1500, 0.5, t, 0.22 * v, 0.01); },
    snare: (d, t, v, tu, dm) => { dTone(d, 'triangle', 190 * tu, 160 * tu, 0.08, t, 0.5 * v, 0.12 * dm); dNoise(d, 'highpass', 1400, 0.6, t, 0.55 * v, 0.2 * dm); },
    clap: (d, t, v, tu, dm) => { for (let i = 0; i < 3; i++) dNoise(d, 'bandpass', 1100 * tu, 1.4, t + i * 0.011, 1.3 * v, 0.014); dNoise(d, 'bandpass', 1100 * tu, 1.4, t + 0.033, 1.4 * v, 0.24 * dm); },
    chat: (d, t, v, tu, dm) => { const o = open.get(d); if (o) { o.gain.cancelScheduledValues(t); o.gain.setTargetAtTime(0, t, 0.008); open.delete(d); } dMetal(d, t, 0.32 * v, 0.06 * dm, tu, 7000, 10000); },
    ohat: (d, t, v, tu, dm) => { open.set(d, dMetal(d, t, 0.3 * v, 0.4 * dm, tu, 7000, 10000)); },
    rim: (d, t, v, tu) => { dTone(d, 'square', 1750 * tu, 1750 * tu, 0, t, 0.22 * v, 0.03); dTone(d, 'sine', 480 * tu, 380 * tu, 0.02, t, 0.5 * v, 0.04); dNoise(d, 'highpass', 2000, 0.5, t, 0.2 * v, 0.02); },
    bell: (d, t, v, tu, dm) => {
      const g = gain(0); g.gain.setValueAtTime(0.28 * v, t); expTo(g.gain, 0.06 * v, t + 0.06); expTo(g.gain, 0.0003, t + 0.5 * dm);
      const b = filt('bandpass', 800 * tu, 1); b.connect(g); g.connect(d);
      [540, 800].forEach((f) => { const o = osc('square', f * tu, t); o.connect(b); o.start(t); o.stop(t + 0.55 * dm + 0.05); });
    },
    tomL: (d, t, v, tu, dm) => dTone(d, 'sine', 160 * tu, 100 * tu, 0.12, t, 0.9 * v, 0.5 * dm),
    tomM: (d, t, v, tu, dm) => dTone(d, 'sine', 240 * tu, 150 * tu, 0.1, t, 0.85 * v, 0.42 * dm),
    tomH: (d, t, v, tu, dm) => dTone(d, 'sine', 340 * tu, 220 * tu, 0.09, t, 0.8 * v, 0.36 * dm),
    conga: (d, t, v, tu, dm) => { dTone(d, 'sine', 420 * tu, 300 * tu, 0.05, t, 0.8 * v, 0.28 * dm); dNoise(d, 'bandpass', 1800, 2, t, 0.2 * v, 0.02); },
    crash: (d, t, v, tu, dm) => { dMetal(d, t, 0.07 * v, 1.5 * dm, tu, 4500, 8000); dNoise(d, 'highpass', 5000, 0.5, t, 0.16 * v, 1.2 * dm); },
    ride: (d, t, v, tu, dm) => { dMetal(d, t, 0.16 * v, 0.9 * dm, tu * 0.7, 3500, 6000); dTone(d, 'sine', 3200 * tu, 3200 * tu, 0, t, 0.08 * v, 0.2); },
    zap: (d, t, v, tu, dm) => dTone(d, 'sawtooth', 4000 * tu, 70 * tu, 0.22 * dm, t, 0.28 * v, 0.3 * dm),
    perc: (d, t, v, tu, dm) => { dTone(d, 'sine', 1300 * tu, 900 * tu, 0.04, t, 0.4 * v, 0.1 * dm); dNoise(d, 'bandpass', 3200 * tu, 8, t, 0.5 * v, 0.07 * dm); },
  };
  A.drum = (name, dest, t, vel = 0.9, tune = 1, decay = 1) => (DRUM[name] || DRUM.perc)(dest, t, vel, tune, decay);
  A.dTone = dTone; A.dNoise = dNoise;

  /* ------------------------------------------------------------------ a lookahead scheduler */
  A.Scheduler = class Scheduler {
    constructor(stepFn, cb) { this.stepFn = stepFn; this.cb = cb; this.on = false; this.step = 0; this.next = 0; this.timer = null; }
    start(from = 0) {
      if (this.on || !A.ctx) return;
      this.on = true; this.step = from; this.next = A.T() + 0.06;
      this.timer = setInterval(() => this.pump(), 25); this.pump();
    }
    stop() { this.on = false; clearInterval(this.timer); }
    pump() {
      if (!this.on) return;
      let guard = 0;
      while (this.next < A.T() + 0.12 && guard++ < 16) {
        this.cb(this.step, this.next);
        this.next += this.stepFn(this.step); this.step++;
      }
    }
  };

  /* ------------------------------------------------------------------ the demo loop effect apps listen to */
  const DL = (A.demoLoop = { on: false, sched: null, bpm: 112 });
  DL.start = function () {
    if (DL.on || !A.ctx) return;
    DL.on = true;
    const ARP = [0, 3, 7, 10, 12, 10, 7, 3], BASS = [0, 0, 7, 0, 5, 0, 10, 3];
    DL.sched = new A.Scheduler(() => 60 / DL.bpm / 4, (s, t) => {
      const d = A.demoBus, st = s % 16;
      if ([0, 4, 8, 12].includes(st) || st === 10) A.drum('kick', d, t, 0.9);
      if (st === 4 || st === 12) A.drum('snare', d, t, 0.7);
      if (st % 2 === 0) A.drum('chat', d, t, st % 4 === 2 ? 0.7 : 0.45);
      if (st === 14) A.drum('ohat', d, t, 0.5);
      if (st % 2 === 0) {
        const v = A.subVoice(d, t, midiHz(33 + BASS[(st / 2) | 0]), 0.9, { wave: 'sawtooth', sub: 0.8, cutoff: 500, envAmt: 2, res: 4, dec: 0.18, sus: 0.3, rel: 0.1, level: 0.5 });
        v.stop(t + 0.2);
      }
      const n = ARP[s % 8] + (Math.floor(s / 32) % 2 ? 5 : 0);
      A.pluckVoice(d, t, midiHz(69 + n), 0.5, { damp: 0.3, t60: 0.9, level: 0.7 });
    });
    DL.sched.start();
  };
  DL.stop = function () { if (DL.sched) DL.sched.stop(); DL.on = false; };


  /* ------------------------------------------------------------------ per-app insert: chorus + echo, then the channel */
  A.insertFx = function insertFx(ch) {
    const c = A.ctx, fx = { in: c.createGain(), out: c.createGain() };
    const dry = c.createGain(); fx.in.connect(dry); dry.connect(fx.out);
    const chor = c.createGain(); chor.gain.value = 0; chor.connect(fx.out);
    [0.018, 0.027].forEach((d, i) => {
      const dl = c.createDelay(0.1); dl.delayTime.value = d;
      const l = c.createOscillator(); l.frequency.value = 0.6 + i * 0.35; const lg = c.createGain(); lg.gain.value = 0.004; l.connect(lg); lg.connect(dl.delayTime); l.start();
      fx.in.connect(dl); dl.connect(chor);
    });
    const dly = c.createDelay(1.2), fbk = c.createGain(), dmix = c.createGain(); dmix.gain.value = 0;
    const lp = c.createBiquadFilter(); lp.type = 'lowpass'; lp.frequency.value = 3600;
    fx.in.connect(dly); dly.connect(lp); lp.connect(fbk); fbk.connect(dly); lp.connect(dmix); dmix.connect(fx.out);
    fx.out.connect(ch.input);
    fx.set = (o) => {
      const t = A.T();
      if (o.chorus != null) chor.gain.setTargetAtTime(o.chorus * 0.7, t, 0.05);
      if (o.dtime != null) dly.delayTime.setTargetAtTime(clamp(o.dtime, 0.02, 1.1), t, 0.05);
      if (o.fb != null) fbk.gain.setTargetAtTime(clamp(o.fb, 0, 0.92), t, 0.05);
      if (o.delay != null) dmix.gain.setTargetAtTime(o.delay, t, 0.05);
      if (o.verb != null) ch.setSend(0.04 + o.verb * 0.5);
    };
    return fx;
  };
  /* wavefolder / soft clipper curve */
  A.shaper = function shaper(amount, fold) {
    const n = 1024, curve = new Float32Array(n);
    for (let i = 0; i < n; i++) { const x = (i / (n - 1)) * 2 - 1; curve[i] = fold ? Math.sin(x * amount * Math.PI / 2) : Math.tanh(x * amount); }
    const w = A.ctx.createWaveShaper(); w.curve = curve; w.oversample = '2x'; return w;
  };

  R.audio = A;
})();
