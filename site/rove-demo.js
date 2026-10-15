/* ROVE browser demo: the control surface (pads, knobs, D-pad, F1-F4, 16:9 screen) plus five
   sample instruments written for Web Audio.

   Honesty note, same rule as the rest of the project: these five voices are small sketches made
   for this page. They are NOT the engines the real apps run (those are native). The page says so.

   Real-time notes: all scheduling uses AudioParam automation against ctx.currentTime, so notes
   are sample-accurate and nothing here runs per-sample on the main thread except Karplus-Strong
   buffer fills, which are short and happen at note-on. */
(() => {
  'use strict';

  const $ = (s, r = document) => r.querySelector(s);
  const root = $('#rove');
  if (!root) return;

  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const reduceMotion = matchMedia('(prefers-reduced-motion: reduce)').matches;

  /* ------------------------------------------------------------------ music helpers */
  const PAD_KEYS = ['1', '2', '3', '4', 'q', 'w', 'e', 'r', 'a', 's', 'd', 'f', 'z', 'x', 'c', 'v'];
  const NOTE_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
  const PENTA = [0, 3, 5, 7, 10];                 // minor pentatonic: every pad is a good note
  const midiHz = (m) => 440 * 2 ** ((m - 69) / 12);
  const noteName = (m) => NOTE_NAMES[m % 12] + (Math.floor(m / 12) - 1);
  // pads are numbered row-major from the top-left; pitch rises left to right, bottom to top
  const padToDegree = (p) => (3 - (p >> 2)) * 4 + (p & 3);
  const degreeToPad = (d) => (3 - (d >> 2)) * 4 + (d & 3);
  const degMidi = (base, d) => base + 12 * Math.floor(d / 5) + PENTA[d % 5];

  /* ------------------------------------------------------------------ audio engine */
  const A = { ctx: null, space: true, volume: 0.75 };
  const volToGain = (v) => v * v * 0.9;

  function initAudio() {
    if (A.ctx) {
      if (A.ctx.state === 'suspended') A.ctx.resume();
      return true;
    }
    const AC = window.AudioContext || window.webkitAudioContext;
    if (!AC) return false;
    const c = (A.ctx = new AC({ latencyHint: 'interactive' }));

    A.in = c.createGain();
    const bus = c.createGain();
    A.master = c.createGain();
    A.master.gain.value = volToGain(A.volume);
    const comp = c.createDynamicsCompressor();
    comp.threshold.value = -14; comp.knee.value = 10; comp.ratio.value = 6; comp.attack.value = 0.003; comp.release.value = 0.2;
    A.analyser = c.createAnalyser();
    A.analyser.fftSize = 2048;
    A.analyser.smoothingTimeConstant = 0.55;
    A.in.connect(bus); bus.connect(A.master);

    // shared echo + room, fed by the SPACE send
    A.send = c.createGain();
    A.send.gain.value = A.space ? 0.3 : 0;
    A.in.connect(A.send);
    const delay = c.createDelay(1); delay.delayTime.value = 0.27;
    const fb = c.createGain(); fb.gain.value = 0.36;
    const fbLp = c.createBiquadFilter(); fbLp.type = 'lowpass'; fbLp.frequency.value = 3200;
    A.send.connect(delay); delay.connect(fbLp); fbLp.connect(fb); fb.connect(delay);
    const delayOut = c.createGain(); delayOut.gain.value = 0.55; delay.connect(delayOut); delayOut.connect(A.master);
    const verb = c.createConvolver();
    const irLen = Math.floor(c.sampleRate * 1.9);
    const ir = c.createBuffer(2, irLen, c.sampleRate);
    for (let ch = 0; ch < 2; ch++) {
      const d = ir.getChannelData(ch);
      for (let i = 0; i < irLen; i++) d[i] = (Math.random() * 2 - 1) * (1 - i / irLen) ** 2.6;
    }
    verb.buffer = ir;
    const verbOut = c.createGain(); verbOut.gain.value = 0.6;
    A.send.connect(verb); delay.connect(verb); verb.connect(verbOut); verbOut.connect(A.master);

    A.master.connect(comp); comp.connect(A.analyser); A.analyser.connect(c.destination);

    const nl = c.sampleRate * 2;
    A.noise = c.createBuffer(1, nl, c.sampleRate);
    const nd = A.noise.getChannelData(0);
    for (let i = 0; i < nl; i++) nd[i] = Math.random() * 2 - 1;
    return true;
  }

  const T = () => A.ctx.currentTime;
  function osc(type, f, t) { const o = A.ctx.createOscillator(); o.type = type; o.frequency.setValueAtTime(f, t); return o; }
  function gain(v) { const g = A.ctx.createGain(); g.gain.value = v; return g; }
  function filt(type, f, q) { const b = A.ctx.createBiquadFilter(); b.type = type; b.frequency.value = f; if (q != null) b.Q.value = q; return b; }
  function noiseSrc() { const s = A.ctx.createBufferSource(); s.buffer = A.noise; s.loop = true; s.loopStart = 0; s.loopEnd = 2; return s; }
  const expTo = (p, v, t) => p.exponentialRampToValueAtTime(Math.max(v, 1e-4), t);

  /* A voice's last stage is a `rel` gain so release never fights the note's own envelope. */
  function releaseVoice(rel, sources, t, time) {
    rel.gain.cancelScheduledValues(t);
    rel.gain.setValueAtTime(1, t);
    rel.gain.setTargetAtTime(0, t, time / 4);
    sources.forEach((s) => { try { s.stop(t + time * 1.8); } catch (_) { /* already stopped */ } });
  }

  /* ------------------------------------------------------------------ the five instruments */
  const pct = (v) => Math.round(v * 100) + '%';
  const secs = (s) => s.toFixed(2) + ' s';
  const hz = (f) => (f >= 1000 ? (f / 1000).toFixed(1) + ' kHz' : Math.round(f) + ' Hz');

  function pitchedLabels(base) {
    return (oct) => Array.from({ length: 16 }, (_, p) => noteName(degMidi(base + 12 * oct, padToDegree(p))));
  }

  const INSTRUMENTS = [
    {
      id: 'bass', name: 'Sub Bass', tag: 'analog · saw + square sub', sustains: true, octaves: true,
      def: [0.45, 0.35], labels: pitchedLabels(45),
      knobs: [
        { name: 'Cutoff', show: (v) => hz(90 * 2 ** (v * 6.5)) },
        { name: 'Resonance', show: pct },
      ],
      start(t, pad, vel, k) {
        const f = midiHz(degMidi(45 + 12 * k.oct, padToDegree(pad)));
        const cut = 90 * 2 ** (k.k1 * 6.5), q = 0.6 + k.k2 * 14;
        const o1 = osc('sawtooth', f, t), o2 = osc('square', f / 2, t), o3 = osc('sawtooth', f, t);
        o3.detune.value = 8;
        const mix = gain(0.2 / (1 + q * 0.12));
        const lp = filt('lowpass', cut, q);
        lp.frequency.setValueAtTime(Math.min(cut * (2 + (1 - k.k1) * 4), 16000), t);
        expTo(lp.frequency, cut, t + 0.22);
        const env = gain(0);
        env.gain.setValueAtTime(0, t);
        env.gain.linearRampToValueAtTime(vel, t + 0.006);
        const rel = gain(1);
        [o1, o2, o3].forEach((o) => { o.connect(mix); o.start(t); o.stop(t + 30); });
        mix.connect(lp); lp.connect(env); env.connect(rel); rel.connect(A.in);
        return {
          stop(t2) { releaseVoice(rel, [o1, o2, o3], t2, 0.2); },
          update(kk) {
            const c2 = 90 * 2 ** (kk.k1 * 6.5);
            lp.frequency.cancelScheduledValues(T()); lp.frequency.setTargetAtTime(c2, T(), 0.03);
            lp.Q.setTargetAtTime(0.6 + kk.k2 * 14, T(), 0.03);
          },
        };
      },
    },
    {
      id: 'fm', name: 'Glass FM', tag: '2-operator FM · glass / e-piano', sustains: false, octaves: true,
      def: [0.5, 0.45], labels: pitchedLabels(57),
      knobs: [
        { name: 'Bright', show: pct },
        { name: 'Decay', show: (v) => secs(0.4 + v * 3.6) },
      ],
      start(t, pad, vel, k) {
        const f = midiHz(degMidi(57 + 12 * k.oct, padToDegree(pad)));
        const dec = 0.4 + k.k2 * 3.6, idx = 0.3 + k.k1 * 5.5;
        const car = osc('sine', f, t), mod = osc('sine', f * 3, t), tine = osc('sine', f * 14, t);
        const mg = gain(0); mg.gain.setValueAtTime(idx * f * 0.9, t); expTo(mg.gain, idx * f * 0.06, t + dec * 0.6);
        const tg = gain(0); tg.gain.setValueAtTime(idx * f * 0.35, t); expTo(tg.gain, 1, t + 0.06);
        const env = gain(0);
        env.gain.setValueAtTime(0, t); env.gain.linearRampToValueAtTime(vel * 0.5, t + 0.003); expTo(env.gain, 0.0003, t + dec);
        const rel = gain(1);
        mod.connect(mg); mg.connect(car.frequency); tine.connect(tg); tg.connect(car.frequency);
        car.connect(env); env.connect(rel); rel.connect(A.in);
        const end = t + dec + 0.1;
        [car, mod, tine].forEach((o) => { o.start(t); o.stop(end); });
        return { stop() {} };                              // bells ring out; note-off is ignored
      },
    },
    {
      id: 'string', name: 'String', tag: 'Karplus-Strong plucked string', sustains: false, octaves: true,
      def: [0.6, 0.5], labels: pitchedLabels(45),
      knobs: [
        { name: 'Tone', show: pct },
        { name: 'Decay', show: (v) => secs(0.4 + v * 5) },
      ],
      start(t, pad, vel, k) {
        const f = midiHz(degMidi(45 + 12 * k.oct, padToDegree(pad)));
        const sr = A.ctx.sampleRate;
        const N = Math.max(2, Math.round(sr / f));
        const b = 0.5 - k.k1 * 0.47;                       // loop-filter blend: 0.5 dark ... 0.03 bright
        const t60 = 0.4 + k.k2 * 5;
        const g = Math.pow(0.001, (N + b) / (sr * t60));   // per-trip loss that gives the chosen T60
        const len = Math.floor(sr * Math.min(5, t60 * 0.8 + 0.4));
        const buf = A.ctx.createBuffer(1, len, sr), d = buf.getChannelData(0);
        let prev = 0, mean = 0;
        for (let i = 0; i < N; i++) { prev = prev * 0.3 + (Math.random() * 2 - 1) * 0.7; d[i] = prev; mean += prev; }
        mean /= N;
        let pk = 0;
        for (let i = 0; i < N; i++) { d[i] -= mean; pk = Math.max(pk, Math.abs(d[i])); }
        for (let i = 0; i < N; i++) d[i] *= 0.75 / (pk || 1);
        for (let i = N; i < len; i++) d[i] = g * ((1 - b) * d[i - N] + b * (i - N - 1 >= 0 ? d[i - N - 1] : 0));
        const src = A.ctx.createBufferSource();
        src.buffer = buf;
        src.playbackRate.value = (sr / (N + b)) / f;       // integer loop length, fine-tuned to exact pitch
        const env = gain(vel * 1.15), rel = gain(1);
        src.connect(env); env.connect(rel); rel.connect(A.in);
        src.start(t);
        return { stop() {} };                              // a plucked string rings out
      },
    },
    {
      id: 'blaster', name: 'Blaster', tag: 'hold to charge · release to fire', sustains: true, octaves: true,
      def: [0.45, 0.45], labels: pitchedLabels(45),
      knobs: [
        { name: 'Charge', show: (v) => secs(0.3 * (2.5 / 0.3) ** v) },
        { name: 'Blast', show: (v) => secs(0.18 + v * 1.2) },
      ],
      start(t, pad, vel, k) {
        const f = midiHz(degMidi(45 + 12 * k.oct, padToDegree(pad)));
        const ct = 0.3 * (2.5 / 0.3) ** k.k1;
        // charging: pitch climbs while a sawtooth LFO makes ever-quicker chirps; a flutter arrives at full
        const body = osc('square', f * 0.5, t);
        body.frequency.linearRampToValueAtTime(f * 1.5, t + ct);
        const lfo = osc('sawtooth', 2.5, t); expTo(lfo.frequency, 20, t + ct);
        const lg = gain(260); lfo.connect(lg); lg.connect(body.detune);
        const lp = filt('lowpass', 1400, 1.2); lp.frequency.linearRampToValueAtTime(5200, t + ct);
        const env = gain(0); env.gain.setValueAtTime(0, t); env.gain.linearRampToValueAtTime(0.2 * vel, t + 0.02);
        const fl = osc('sine', 27, t), fg = gain(0);
        fg.gain.setValueAtTime(0, t + ct); fg.gain.linearRampToValueAtTime(0.1 * vel, t + ct + 0.05);
        fl.connect(fg); fg.connect(env.gain);
        const rel = gain(1);
        body.connect(lp); lp.connect(env); env.connect(rel); rel.connect(A.in);
        const srcs = [body, lfo, fl];
        srcs.forEach((o) => { o.start(t); o.stop(t + 30); });
        return {
          stop(t2) {
            const c = clamp((t2 - t) / ct, 0.1, 1);
            releaseVoice(rel, srcs, t2, 0.03);
            fire(t2, c, f, vel, k);
          },
        };
      },
    },
    {
      id: 'drums', name: 'Drums', tag: 'synthesised kit · kick, snare, metal hats', sustains: false, octaves: false,
      def: [0.5, 0.5],
      labels: () => ['CRASH', 'RIDE', 'ZAP', 'PERC', 'TOM L', 'TOM M', 'TOM H', 'CONGA', 'HAT', 'O-HAT', 'RIM', 'BELL', 'KICK', 'KICK 2', 'SNARE', 'CLAP'],
      knobs: [
        { name: 'Tune', show: (v) => ((v - 0.5) * 14).toFixed(1) + ' st' },
        { name: 'Decay', show: (v) => (2 ** ((v - 0.5) * 2.4)).toFixed(2) + '×' },
      ],
      start(t, pad, vel, k) {
        const tune = 2 ** (((k.k1 - 0.5) * 14) / 12), dm = 2 ** ((k.k2 - 0.5) * 2.4);
        DRUMS[pad](t, vel, tune, dm);
        return { stop() {} };
      },
    },
  ];

  /* blaster release: the shot */
  function fire(t, c, f, vel, k) {
    const len = (0.18 + k.k2 * 1.2) * (0.55 + 0.45 * c);
    const amp = 0.5 * (0.35 + 0.65 * c) * vel;
    const o = osc('sawtooth', f * (1.5 + 2.5 * c), t); expTo(o.frequency, Math.max(f * 0.3, 30), t + len * 0.7);
    const sub = osc('sine', f * 0.5 * (1 + c), t); expTo(sub.frequency, 32, t + len * 0.8);
    const lp = filt('lowpass', 7000, 0.8); lp.frequency.setValueAtTime(7000, t); expTo(lp.frequency, 500, t + len);
    const g = gain(0); g.gain.setValueAtTime(amp, t); expTo(g.gain, 0.0003, t + len);
    const sg = gain(0); sg.gain.setValueAtTime(amp * 1.2, t); expTo(sg.gain, 0.0003, t + len * 0.9);
    o.connect(lp); lp.connect(g); g.connect(A.in); sub.connect(sg); sg.connect(A.in);
    o.start(t); sub.start(t); o.stop(t + len + 0.05); sub.stop(t + len + 0.05);
    dNoise('bandpass', 5000, 0.9, t, 0.1 + 0.4 * c, len * 0.5, 350);
  }

  /* ---- drum building blocks ---- */
  function dTone(type, f0, f1, sweep, t, peak, decay) {
    const o = osc(type, f0, t);
    if (f1 !== f0) expTo(o.frequency, f1, t + sweep);
    const g = gain(0); g.gain.setValueAtTime(peak, t); expTo(g.gain, 0.0003, t + decay);
    o.connect(g); g.connect(A.in); o.start(t); o.stop(t + decay + 0.05);
    return g;
  }
  function dNoise(type, f, q, t, peak, decay, f2) {
    const s = noiseSrc(), fl = filt(type, f, q);
    if (f2) expTo(fl.frequency, f2, t + decay);
    const g = gain(0); g.gain.setValueAtTime(peak, t); expTo(g.gain, 0.0003, t + decay);
    s.connect(fl); fl.connect(g); g.connect(A.in); s.start(t, Math.random()); s.stop(t + decay + 0.05);
    return g;
  }
  // classic six-square metallic cluster used for hats and cymbals
  const METAL = [205.3, 304.4, 369.6, 522.7, 540, 800];
  function dMetal(t, peak, decay, tune, hp, bp) {
    const g = gain(0); g.gain.setValueAtTime(peak, t); expTo(g.gain, 0.0003, t + decay);
    const b = filt('bandpass', bp, 0.7), h = filt('highpass', hp, 0.8);
    b.connect(h); h.connect(g); g.connect(A.in);
    METAL.forEach((f) => { const o = osc('square', f * tune * 2, t); const m = gain(0.7); o.connect(m); m.connect(b); o.start(t); o.stop(t + decay + 0.05); });
    return g;
  }
  let openHat = null;
  const DRUMS = [
    /* 0 crash  */ (t, v, tu, dm) => { dMetal(t, 0.07 * v, 1.5 * dm, tu, 4500, 8000); dNoise('highpass', 5000, 0.5, t, 0.16 * v, 1.2 * dm); },
    /* 1 ride   */ (t, v, tu, dm) => { dMetal(t, 0.16 * v, 0.9 * dm, tu * 0.7, 3500, 6000); dTone('sine', 3200 * tu, 3200 * tu, 0, t, 0.08 * v, 0.2); },
    /* 2 zap    */ (t, v, tu, dm) => { dTone('sawtooth', 4000 * tu, 70 * tu, 0.22 * dm, t, 0.28 * v, 0.3 * dm); },
    /* 3 perc   */ (t, v, tu, dm) => { dTone('sine', 1300 * tu, 900 * tu, 0.04, t, 0.4 * v, 0.1 * dm); dNoise('bandpass', 3200 * tu, 8, t, 0.5 * v, 0.07 * dm); },
    /* 4 tom lo */ (t, v, tu, dm) => { dTone('sine', 160 * tu, 100 * tu, 0.12, t, 0.9 * v, 0.5 * dm); },
    /* 5 tom md */ (t, v, tu, dm) => { dTone('sine', 240 * tu, 150 * tu, 0.1, t, 0.85 * v, 0.42 * dm); },
    /* 6 tom hi */ (t, v, tu, dm) => { dTone('sine', 340 * tu, 220 * tu, 0.09, t, 0.8 * v, 0.36 * dm); },
    /* 7 conga  */ (t, v, tu, dm) => { dTone('sine', 420 * tu, 300 * tu, 0.05, t, 0.8 * v, 0.28 * dm); dNoise('bandpass', 1800, 2, t, 0.2 * v, 0.02); },
    /* 8 c-hat  */ (t, v, tu, dm) => {
      if (openHat) { openHat.gain.cancelScheduledValues(t); openHat.gain.setTargetAtTime(0, t, 0.008); openHat = null; }   // closed hat chokes the open one
      dMetal(t, 0.32 * v, 0.06 * dm, tu, 7000, 10000);
    },
    /* 9 o-hat  */ (t, v, tu, dm) => { openHat = dMetal(t, 0.3 * v, 0.4 * dm, tu, 7000, 10000); },
    /* 10 rim   */ (t, v, tu) => { dTone('square', 1750 * tu, 1750 * tu, 0, t, 0.22 * v, 0.03); dTone('sine', 480 * tu, 380 * tu, 0.02, t, 0.5 * v, 0.04); dNoise('highpass', 2000, 0.5, t, 0.2 * v, 0.02); },
    /* 11 bell  */ (t, v, tu, dm) => {
      const g = gain(0); g.gain.setValueAtTime(0.28 * v, t); expTo(g.gain, 0.06 * v, t + 0.06); expTo(g.gain, 0.0003, t + 0.5 * dm);
      const b = filt('bandpass', 800 * tu, 1); b.connect(g); g.connect(A.in);
      [540, 800].forEach((f) => { const o = osc('square', f * tu, t); o.connect(b); o.start(t); o.stop(t + 0.55 * dm + 0.05); });
    },
    /* 12 kick  */ (t, v, tu, dm) => { dTone('sine', 160 * tu, 45 * tu, 0.11, t, 1.0 * v, 0.55 * dm); dNoise('lowpass', 3000, 0.5, t, 0.2 * v, 0.012); },
    /* 13 kick2 */ (t, v, tu, dm) => { dTone('sine', 240 * tu, 55 * tu, 0.07, t, 1.0 * v, 0.4 * dm); dTone('triangle', 90 * tu, 50 * tu, 0.1, t, 0.4 * v, 0.2 * dm); dNoise('highpass', 1500, 0.5, t, 0.22 * v, 0.01); },
    /* 14 snare */ (t, v, tu, dm) => { dTone('triangle', 190 * tu, 160 * tu, 0.08, t, 0.5 * v, 0.12 * dm); dNoise('highpass', 1400, 0.6, t, 0.55 * v, 0.2 * dm); },
    /* 15 clap  */ (t, v, tu, dm) => {
      for (let i = 0; i < 3; i++) dNoise('bandpass', 1100 * tu, 1.4, t + i * 0.011, 1.3 * v, 0.014);
      dNoise('bandpass', 1100 * tu, 1.4, t + 0.033, 1.4 * v, 0.24 * dm);
    },
  ];

  /* ------------------------------------------------------------------ demo loop patterns */
  // [step, degree-or-pad, lengthInSteps], 32 sixteenth-note steps at 108 BPM; everything is A minor pentatonic
  const BPM = 108, STEPS = 32;
  const PATTERNS = {
    bass: [[0, 0, 2], [3, 0, 1], [4, 3, 2], [7, 4, 1], [8, 0, 2], [11, 5, 1], [12, 4, 2], [14, 2, 1], [16, 0, 2], [19, 0, 1], [20, 3, 2], [23, 4, 1], [24, 2, 2], [27, 3, 1], [28, 5, 3]],
    fm: [[0, 5, 2], [2, 8, 2], [4, 7, 2], [6, 10, 2], [8, 9, 2], [10, 7, 2], [12, 8, 2], [14, 6, 2], [16, 5, 2], [18, 8, 2], [20, 10, 2], [22, 12, 2], [24, 11, 2], [26, 10, 2], [28, 8, 2], [30, 9, 2]],
    string: [[0, 5, 3], [3, 7, 1], [4, 8, 2], [6, 7, 2], [8, 10, 4], [14, 8, 2], [16, 9, 3], [19, 7, 1], [20, 5, 2], [22, 7, 2], [24, 3, 3], [27, 5, 1], [28, 8, 4]],
    blaster: [[0, 4, 3], [4, 7, 1], [6, 7, 1], [8, 5, 5], [14, 9, 1], [16, 4, 2], [19, 8, 1], [22, 6, 1], [24, 10, 6]],
    drums: [
      ...[0, 7, 10, 16, 23, 26].map((s) => [s, 12, 1]), [0, 13, 1], [8, 14, 1], [24, 14, 1], [31, 14, 1], [8, 15, 1], [24, 15, 1],
      ...Array.from({ length: 16 }, (_, i) => [i * 2, 8, 1]).filter(([s]) => s !== 14 && s !== 30), [14, 9, 1], [30, 9, 1],
      [3, 10, 1], [19, 10, 1], [12, 11, 1], [5, 7, 1], [13, 7, 1], [21, 7, 1], [26, 4, 1], [28, 5, 1], [29, 5, 1], [30, 6, 1], [0, 0, 1],
    ],
  };

  /* ------------------------------------------------------------------ state */
  const S = {
    i: 0, oct: 0,
    k: INSTRUMENTS.map((x) => x.def.slice()),
    demo: false, step: 0, next: 0, timer: null,
    keys: false,                       // computer-keyboard play is armed
  };
  const inst = () => INSTRUMENTS[S.i];
  const kvals = () => ({ k1: S.k[S.i][0], k2: S.k[S.i][1], oct: S.oct });

  const held = new Map();            // key -> { pad, voice }
  const demoLights = [];             // { pad, t0, t1 }
  const live = $('#live');
  const say = (msg) => { if (live) live.textContent = msg; };

  /* ------------------------------------------------------------------ pads */
  const padsEl = $('#pads');
  const padEls = [];
  for (let p = 0; p < 16; p++) {
    const b = document.createElement('button');
    b.type = 'button'; b.className = 'pad'; b.dataset.row = String(p >> 2); b.dataset.pad = String(p);
    b.setAttribute('aria-keyshortcuts', PAD_KEYS[p].toUpperCase());
    b.innerHTML = '<span aria-hidden="true"></span>';
    padsEl.appendChild(b);
    padEls.push(b);

    b.addEventListener('pointerdown', (e) => {
      if (e.button !== 0 && e.pointerType === 'mouse') return;
      e.preventDefault();
      try { b.setPointerCapture(e.pointerId); } catch (_) { /* synthetic events */ }
      b.focus({ preventScroll: true });
      press(p, 'ptr' + e.pointerId);
    });
    const up = (e) => release('ptr' + e.pointerId);
    b.addEventListener('pointerup', up);
    b.addEventListener('pointercancel', up);
    b.addEventListener('lostpointercapture', up);
    b.addEventListener('contextmenu', (e) => e.preventDefault());
    b.addEventListener('keydown', (e) => {
      if ((e.key === ' ' || e.key === 'Enter') && !e.repeat) { e.preventDefault(); press(p, 'kbd' + p); }
    });
    b.addEventListener('keyup', (e) => { if (e.key === ' ' || e.key === 'Enter') release('kbd' + p); });
  }

  function labelPads() {
    const labels = inst().labels(S.oct);
    padEls.forEach((el, p) => {
      el.firstChild.textContent = labels[p];
      el.setAttribute('aria-label', `Pad ${p + 1}, ${labels[p]}`);
    });
  }

  function press(pad, key) {
    if (held.has(key)) return;
    if (!initAudio()) { say('Sorry, this browser has no Web Audio support.'); return; }
    hideGate();
    const voice = inst().start(T() + 0.004, pad, 0.85, kvals());
    held.set(key, { pad, voice });
    syncLights();
  }
  function release(key) {
    const h = held.get(key);
    if (!h) return;
    held.delete(key);
    if (A.ctx) h.voice.stop(T());
    syncLights();
  }
  function releaseAll() { [...held.keys()].forEach(release); }

  let lastLit = '';
  function syncLights() {
    const t = A.ctx ? T() : 0, on = new Set();
    held.forEach((h) => on.add(h.pad));
    for (let i = demoLights.length - 1; i >= 0; i--) {
      const d = demoLights[i];
      if (t > d.t1 + 0.05) demoLights.splice(i, 1);
      else if (t >= d.t0 && t <= d.t1) on.add(d.pad);
    }
    const sig = [...on].sort((a, b) => a - b).join(',');
    if (sig === lastLit) return;
    lastLit = sig;
    padEls.forEach((el, p) => { if (on.has(p)) el.setAttribute('data-on', ''); else el.removeAttribute('data-on'); });
  }
  const litPads = () => new Set(lastLit ? lastLit.split(',').map(Number) : []);

  /* ------------------------------------------------------------------ knobs */
  const knobs = [0, 1].map((i) => {
    const el = $('#knob' + i);
    const track = $('.knob__track', el), arc = $('.knob__arc', el), cap = $('.knob__cap', el);
    const label = $('.knob__label', el), val = $('.knob__value', el);
    const R = 42, A0 = -135, A1 = 135;
    const pt = (a) => { const r = ((a - 90) * Math.PI) / 180; return [50 + R * Math.cos(r), 50 + R * Math.sin(r)]; };
    const arcPath = (a, b) => { const [x0, y0] = pt(a), [x1, y1] = pt(b); return `M${x0.toFixed(2)} ${y0.toFixed(2)}A${R} ${R} 0 ${b - a > 180 ? 1 : 0} 1 ${x1.toFixed(2)} ${y1.toFixed(2)}`; };
    track.setAttribute('d', arcPath(A0, A1));
    const get = () => S.k[S.i][i];

    function render() {
      const v = get(), k = inst().knobs[i], a = A0 + (A1 - A0) * v;
      arc.setAttribute('d', v < 0.004 ? '' : arcPath(A0, a));
      cap.setAttribute('transform', `rotate(${a} 50 50)`);
      label.textContent = k.name; val.textContent = k.show(v);
      el.setAttribute('aria-valuenow', String(Math.round(v * 100)));
      el.setAttribute('aria-valuetext', k.show(v));
      el.setAttribute('aria-label', `Knob ${i + 1}: ${k.name}`);
    }
    function set(v) {
      S.k[S.i][i] = clamp(v, 0, 1);
      render();
      held.forEach((h) => { if (h.voice.update) h.voice.update(kvals()); });
    }

    let sy = 0, sv = 0, drag = false;
    el.addEventListener('pointerdown', (e) => { drag = true; sy = e.clientY; sv = get(); el.setPointerCapture(e.pointerId); el.focus({ preventScroll: true }); e.preventDefault(); });
    el.addEventListener('pointermove', (e) => { if (drag) set(sv + (sy - e.clientY) / (e.shiftKey ? 700 : 190)); });
    const end = () => { drag = false; };
    el.addEventListener('pointerup', end); el.addEventListener('pointercancel', end);
    el.addEventListener('wheel', (e) => { e.preventDefault(); set(get() - Math.sign(e.deltaY) * 0.03); }, { passive: false });
    el.addEventListener('dblclick', () => set(inst().def[i]));
    el.addEventListener('keydown', (e) => {
      const step = e.shiftKey ? 0.1 : 0.02;
      const d = { ArrowUp: step, ArrowRight: step, ArrowDown: -step, ArrowLeft: -step, PageUp: 0.1, PageDown: -0.1 }[e.key];
      if (d != null) { e.preventDefault(); e.stopPropagation(); set(get() + d); }
      else if (e.key === 'Home') { e.preventDefault(); set(0); } else if (e.key === 'End') { e.preventDefault(); set(1); }
    });
    return { set, get, render };
  });

  /* ------------------------------------------------------------------ instrument picker, D-pad, F keys */
  const picker = $('#picker');
  INSTRUMENTS.forEach((x, i) => {
    const b = document.createElement('button');
    b.type = 'button'; b.setAttribute('aria-pressed', 'false');
    b.innerHTML = `<span class="n" aria-hidden="true">0${i + 1}</span>${x.name}`;
    b.addEventListener('click', () => setInstrument(i, true));
    picker.appendChild(b);
  });

  function setInstrument(i, announce) {
    releaseAll();
    S.i = ((i % INSTRUMENTS.length) + INSTRUMENTS.length) % INSTRUMENTS.length;
    if (!inst().octaves) S.oct = 0;
    [...picker.children].forEach((b, n) => b.setAttribute('aria-pressed', String(n === S.i)));
    labelPads(); knobs.forEach((k) => k.render());
    if (announce) say(`${inst().name}. ${inst().tag}. Knob 1 ${inst().knobs[0].name}, knob 2 ${inst().knobs[1].name}.`);
  }
  function setOctave(d) {
    if (!inst().octaves) { say('Drums have no octave.'); return; }
    releaseAll();
    S.oct = clamp(S.oct + d, -1, 1);
    labelPads();
    say(`Octave ${S.oct > 0 ? '+' : ''}${S.oct}`);
  }
  root.querySelectorAll('.dpad button').forEach((b) => b.addEventListener('click', () => {
    ({ left: () => setInstrument(S.i - 1, true), right: () => setInstrument(S.i + 1, true), up: () => setOctave(1), down: () => setOctave(-1) })[b.dataset.dir]();
  }));

  const fkeys = [...root.querySelectorAll('.fkey')];
  const fkey = (n) => fkeys[n - 1];
  fkey(1).addEventListener('click', () => toggleDemo());
  fkey(2).addEventListener('click', () => toggleSpace());
  fkey(3).addEventListener('click', () => { S.k[S.i] = S.k[S.i].map(() => 0.1 + Math.random() * 0.8); knobs.forEach((k) => k.set(k.get())); say('Knobs randomised.'); });
  fkey(4).addEventListener('click', () => { S.k[S.i] = inst().def.slice(); S.oct = 0; labelPads(); knobs.forEach((k) => k.set(k.get())); say('Reset to defaults.'); });

  function toggleSpace() {
    A.space = !A.space;
    fkey(2).setAttribute('aria-pressed', String(A.space));
    if (A.ctx) A.send.gain.setTargetAtTime(A.space ? 0.3 : 0, T(), 0.03);
    say(A.space ? 'Space on.' : 'Space off.');
  }

  /* ------------------------------------------------------------------ demo loop scheduler */
  const demoBtn = $('#demoBtn');
  const stepDur = 60 / BPM / 4;
  function scheduleStep(step, t) {
    const x = inst();
    PATTERNS[x.id].forEach(([s, n, len]) => {
      if (s !== step) return;
      const pad = x.id === 'drums' ? n : degreeToPad(n);
      const dur = Math.max(len * stepDur * 0.92, 0.05);
      const v = x.start(t, pad, 0.8, kvals());
      v.stop(t + dur);
      demoLights.push({ pad, t0: t, t1: t + Math.max(dur, 0.12) });
    });
  }
  function pump() {
    if (!S.demo || !A.ctx) return;
    while (S.next < T() + 0.12) {
      scheduleStep(S.step, S.next);
      S.next += stepDur;
      S.step = (S.step + 1) % STEPS;
    }
  }
  function toggleDemo(force) {
    const on = force != null ? force : !S.demo;
    if (on === S.demo) return;
    if (on) {
      if (!initAudio()) return;
      hideGate();
      S.demo = true; S.step = 0; S.next = T() + 0.06;
      S.timer = setInterval(pump, 25);
      pump();
    } else {
      S.demo = false; clearInterval(S.timer);
    }
    demoBtn.setAttribute('aria-pressed', String(S.demo));
    demoBtn.querySelector('span').textContent = S.demo ? 'Stop demo loop' : 'Play a demo loop';
    fkey(1).setAttribute('aria-pressed', String(S.demo));
    say(S.demo ? 'Demo loop playing. Switch instruments to hear it change.' : 'Demo loop stopped.');
  }
  demoBtn.addEventListener('click', () => toggleDemo());

  /* ------------------------------------------------------------------ volume + sound gate */
  $('#vol').addEventListener('input', (e) => {
    A.volume = e.target.value / 100;
    if (A.ctx) A.master.gain.setTargetAtTime(volToGain(A.volume), T(), 0.02);
  });
  const gate = $('#gate');
  function hideGate() { gate.hidden = true; }
  gate.addEventListener('click', () => { if (initAudio()) { hideGate(); press(degreeToPad(5), 'gate'); setTimeout(() => release('gate'), 380); } });

  /* ------------------------------------------------------------------ computer keyboard */
  // Single-key shortcuts only work while the device is "armed" (focused or just clicked),
  // so they never steal keys from the rest of the page or from screen readers.
  const setArmed = (v) => { S.keys = v; };
  root.addEventListener('pointerdown', () => setArmed(true));
  root.addEventListener('focusin', () => setArmed(true));
  document.addEventListener('pointerdown', (e) => { if (!root.contains(e.target)) setArmed(false); });
  document.addEventListener('focusin', (e) => { if (!root.contains(e.target)) setArmed(false); });

  document.addEventListener('keydown', (e) => {
    if (!S.keys || e.ctrlKey || e.metaKey || e.altKey) return;
    if (e.key === 'Escape') { setArmed(false); releaseAll(); return; }
    const k = e.key.toLowerCase(), idx = PAD_KEYS.indexOf(k);
    if (idx >= 0) { e.preventDefault(); if (!e.repeat) press(idx, 'key' + k); return; }
    const nudge = { '[': [0, -0.04], ']': [0, 0.04], ',': [1, -0.04], '.': [1, 0.04] }[e.key];
    if (nudge) { e.preventDefault(); knobs[nudge[0]].set(knobs[nudge[0]].get() + nudge[1]); return; }
    if (document.activeElement && document.activeElement.getAttribute('role') === 'slider') return;
    const nav = { ArrowLeft: () => setInstrument(S.i - 1, true), ArrowRight: () => setInstrument(S.i + 1, true), ArrowUp: () => setOctave(1), ArrowDown: () => setOctave(-1) }[e.key];
    if (nav) { e.preventDefault(); nav(); }
  });
  document.addEventListener('keyup', (e) => { const k = e.key.toLowerCase(); if (PAD_KEYS.includes(k)) release('key' + k); });
  window.addEventListener('blur', releaseAll);

  /* ------------------------------------------------------------------ screen */
  const cv = $('#screen'), g = cv.getContext('2d');
  const C = {};
  function readColors() {
    const cs = getComputedStyle(document.documentElement);
    ['--s1', '--s2', '--s3', '--s4', '--on-s1', '--on-s2', '--on-s3', '--on-s4', '--screen', '--screen-text'].forEach((k) => { C[k] = cs.getPropertyValue(k).trim(); });
  }
  document.addEventListener('rove:palette', readColors);
  readColors();

  // same letterforms as the SVG logo symbols in index.html
  const O = 'M15.5 6.5H44.5A9 9 0 0 1 53.5 15.5V44.5A9 9 0 0 1 44.5 53.5H15.5A9 9 0 0 1 6.5 44.5V15.5A9 9 0 0 1 15.5 6.5Z';
  const GLYPHS = {
    R: 'M6.5 60V6.5H53.5V30H6.5M26 30L56 66', O, V: 'M-4-6L30 56L64-6', E: 'M60 6.5H6.5V53.5H60M6.5 30H50',
  };
  const GP = Object.fromEntries(Object.entries(GLYPHS).map(([k, v]) => [k, new Path2D(v)]));

  function rr(x, y, w, h, r) {
    g.beginPath(); g.moveTo(x + r, y); g.arcTo(x + w, y, x + w, y + h, r); g.arcTo(x + w, y + h, x, y + h, r); g.arcTo(x, y + h, x, y, r); g.arcTo(x, y, x + w, y, r); g.closePath();
  }
  function glyph(ch, x, y, s, color) {
    g.save(); g.translate(x, y); g.scale(s / 60, s / 60); g.beginPath(); g.rect(0, 0, 60, 60); g.clip();
    g.strokeStyle = color; g.lineWidth = 13; g.lineJoin = 'miter'; g.miterLimit = 8; g.lineCap = 'butt'; g.stroke(GP[ch]); g.restore();
  }
  const MONO = '"JetBrains Mono", ui-monospace, Menlo, monospace';
  const DISP = 'Rubik, "Helvetica Neue", Arial, sans-serif';
  const withAlpha = (hex, a) => hex + Math.round(a * 255).toString(16).padStart(2, '0');

  const tbuf = new Float32Array(2048), fbuf = new Uint8Array(1024);
  let frame = 0;

  function drawScreen() {
    frame++;
    const W = 640, H = 360, ink = C['--screen-text'], on = !!A.ctx && A.ctx.state === 'running';
    g.setTransform(2, 0, 0, 2, 0, 0);
    g.fillStyle = C['--screen']; g.fillRect(0, 0, W, H);

    // top bar: logo + status chips
    ['R', 'O', 'V', 'E'].forEach((ch, i) => {
      const x = 24 + i * 25, col = C['--s' + (i + 1)];
      g.fillStyle = col; rr(x, 14, 22, 22, 5); g.fill(); glyph(ch, x + 4.5, 18.5, 13, C['--on-s' + (i + 1)]);
    });
    const chips = [];
    if (S.keys) chips.push('KEYS');
    chips.push(`OCT ${inst().octaves ? (S.oct > 0 ? '+' : '') + S.oct : '–'}`);
    if (A.space) chips.push('SPACE');
    if (S.demo) chips.push(`DEMO ${BPM}`);
    g.font = `700 11px ${MONO}`; g.textBaseline = 'middle';
    let cx = W - 24;
    for (let i = chips.length - 1; i >= 0; i--) {
      const w = g.measureText(chips[i]).width + 18;
      cx -= w;
      const hot = chips[i].startsWith('DEMO');
      g.fillStyle = hot ? C['--s3'] : 'transparent'; g.strokeStyle = hot ? C['--s3'] : withAlpha(ink, 0.45); g.lineWidth = 1.5;
      rr(cx, 16, w, 18, 9); if (hot) g.fill(); g.stroke();
      g.fillStyle = hot ? C['--on-s3'] : ink; g.textAlign = 'center'; g.fillText(chips[i], cx + w / 2, 25.5);
      cx -= 8;
    }

    // title
    g.textAlign = 'left'; g.textBaseline = 'alphabetic';
    g.fillStyle = ink; g.font = `800 36px ${DISP}`; g.fillText(inst().name, 24, 84);
    g.fillStyle = withAlpha(ink, 0.7); g.font = `500 12px ${MONO}`; g.fillText(`APP ${S.i + 1}/${INSTRUMENTS.length}  ·  ${inst().tag.toUpperCase()}`, 24, 104);

    // scope
    const sx = 24, sy = 116, sw = 368, sh = 136;
    g.strokeStyle = withAlpha(ink, 0.3); g.lineWidth = 1.5; rr(sx, sy, sw, sh, 8); g.stroke();
    g.strokeStyle = withAlpha(ink, 0.12); g.beginPath(); g.moveTo(sx + 8, sy + sh / 2); g.lineTo(sx + sw - 8, sy + sh / 2); g.stroke();
    if (on) {
      A.analyser.getFloatTimeDomainData(tbuf);
      let start = 0;
      for (let i = 1; i < 1024; i++) if (tbuf[i - 1] <= 0 && tbuf[i] > 0) { start = i; break; }
      let pk = 0;
      for (let i = 0; i < 768; i++) pk = Math.max(pk, Math.abs(tbuf[start + i]));
      const sc = pk < 0.002 ? 0 : (0.42 * sh) / Math.max(pk, 0.18);
      g.save(); rr(sx, sy, sw, sh, 8); g.clip();
      g.strokeStyle = C['--s2']; g.lineWidth = 2.5; g.lineJoin = 'round'; g.shadowColor = C['--s2']; g.shadowBlur = reduceMotion ? 0 : 8;
      g.beginPath();
      for (let i = 0; i < 768; i++) {
        const x = sx + 8 + (i / 767) * (sw - 16), y = sy + sh / 2 - tbuf[start + i] * sc;
        i ? g.lineTo(x, y) : g.moveTo(x, y);
      }
      g.stroke(); g.restore();
    } else {
      g.fillStyle = withAlpha(ink, 0.85); g.font = `700 13px ${MONO}`; g.textAlign = 'center';
      g.fillText('SOUND OFF', sx + sw / 2, sy + sh / 2 - 8);
      g.fillStyle = withAlpha(ink, 0.6); g.font = `500 11px ${MONO}`; g.fillText('tap a pad or press play', sx + sw / 2, sy + sh / 2 + 12);
      g.textAlign = 'left';
    }

    // pad map
    const lit = litPads(), labels = inst().labels(S.oct);
    for (let p = 0; p < 16; p++) {
      const r = p >> 2, c = p & 3, x = 416 + c * 50, y = 119 + r * 34, col = C['--s' + (r + 1)];
      if (lit.has(p)) { g.fillStyle = col; rr(x, y, 44, 28, 6); g.fill(); }
      else { g.strokeStyle = withAlpha(col, 0.75); g.lineWidth = 2; rr(x + 1, y + 1, 42, 26, 6); g.stroke(); }
      g.fillStyle = lit.has(p) ? C['--on-s' + (r + 1)] : withAlpha(ink, 0.85);
      g.font = `700 ${labels[p].length > 4 ? 9 : 11}px ${MONO}`; g.textAlign = 'center'; g.textBaseline = 'middle';
      g.fillText(labels[p], x + 22, y + 15);
    }
    g.textBaseline = 'alphabetic';

    // spectrum
    const bars = 40, bx = 24, bw = (592 - (bars - 1) * 3) / bars;
    if (on) A.analyser.getByteFrequencyData(fbuf);
    const nyq = A.ctx ? A.ctx.sampleRate / 2 : 24000;
    for (let i = 0; i < bars; i++) {
      let v = 0;
      if (on) {
        const f0 = 40 * (12000 / 40) ** (i / bars), f1 = 40 * (12000 / 40) ** ((i + 1) / bars);
        const b0 = Math.floor((f0 / nyq) * 1024), b1 = Math.max(b0 + 1, Math.ceil((f1 / nyq) * 1024));
        for (let b = b0; b < b1; b++) v = Math.max(v, fbuf[b]);
        v /= 255;
      }
      const h = 3 + v * 44, x = bx + i * (bw + 3);
      g.fillStyle = C['--s' + (i < 14 ? 1 : i < 28 ? 2 : 3)];
      rr(x, 312 - h, bw, h, 2); g.fill();
    }

    // knob read-outs
    [0, 1].forEach((i) => {
      const x = 24 + i * 304, v = S.k[S.i][i], k = inst().knobs[i];
      g.fillStyle = withAlpha(ink, 0.7); g.font = `700 11px ${MONO}`; g.textAlign = 'left'; g.fillText(`KNOB ${i + 1} · ${k.name.toUpperCase()}`, x, 333);
      g.fillStyle = ink; g.textAlign = 'right'; g.fillText(k.show(v), x + 288, 333);
      g.fillStyle = withAlpha(ink, 0.2); rr(x, 340, 288, 7, 3.5); g.fill();
      g.fillStyle = C['--s' + (i ? 3 : 2)]; rr(x, 340, Math.max(7, 288 * v), 7, 3.5); g.fill();
    });
    g.textAlign = 'left';
  }

  let visible = true;
  function loop() {
    try {
      if (visible && !document.hidden) { syncLights(); drawScreen(); }
    } catch (err) { console.error('ROVE screen:', err); }
    requestAnimationFrame(loop);
  }
  new IntersectionObserver(([e]) => {
    visible = e.isIntersecting;
    if (!visible && S.demo) toggleDemo(false);    // don't keep playing once the device scrolls away
  }, { threshold: 0 }).observe(root);
  document.addEventListener('visibilitychange', () => {
    if (document.hidden) { toggleDemo(false); releaseAll(); if (A.ctx) A.ctx.suspend(); } else if (A.ctx) A.ctx.resume();
  });

  /* ------------------------------------------------------------------ go */
  setInstrument(0, false);
  fkey(2).setAttribute('aria-pressed', 'true');
  if (document.fonts && document.fonts.ready) document.fonts.ready.then(drawScreen);
  loop();

  // handy for tests and for tweaking from the console
  window.ROVE = Object.assign(window.ROVE || {}, { demo: { S, INSTRUMENTS, setInstrument, press, release, toggleDemo, initAudio, A, T, scheduleStep, stepDur, STEPS } });
})();
