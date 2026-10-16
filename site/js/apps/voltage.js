/* Voltage: a subtractive polysynth. Controls, presets and the four live panels follow the real app. */
(() => {
  'use strict';
  const R = window.ROVE, A = R.audio, { ctl, c565, fmt } = R;
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const PAL = R.slint.pal('#101819', '#DEF3EF', '#4ACEA5', '#4A6963');
  const WAVES = ['sawtooth', 'square', 'triangle', 'sine'], WAVE_NAMES = ['Saw', 'Square', 'Triangle', 'Sine'];
  const cutoffHz = (v) => 40 * (12000 / 40) ** clamp(v, 0, 1);
  const PRESETS = R.data.voltage;
  const pre = (n) => String(n + 1).padStart(2, '0');

  R.registerApp({
    id: 'voltage', name: 'Voltage', category: 'instrument',
    desc: 'A subtractive polysynth: two oscillators, a sub, a resonant filter with its own envelope, an LFO, chorus, echo and an arpeggiator. Twenty factory presets.',
    create: R.playApp({
      name: 'Voltage', pal: PAL, crumb: 'POLYSYNTH',
      kit(inst) {
        const p = (inst.p = { ...PRESETS[0] });
        const sync = () => { Object.assign(p, { osc1_wave: c[8].value, osc2_wave: c[9].value, osc2_detune: c[10].value, osc_mix: c[3].value, sub_level: c[11].value, noise_level: c[12].value, unison: c[13].value, octave: c[14].value, filter_cutoff: c[0].value, filter_resonance: c[1].value, filter_env_amount: c[2].value, amp_attack: c[4].value, amp_release: c[5].value, lfo_depth: c[6].value, lfo_rate: c[7].value }); update(inst); };
        const f = (label, min, max, def, fm, o = {}) => ctl.f(label, min, max, def, { onChange: sync, fmt: fm, ...o });
        const sec = (v) => fmt.ms(v * 1000), sgn = (v) => `${v >= 0 ? '+' : ''}${v.toFixed(2)}`;
        const c = [
          f('Cutoff', 0, 1, 0.6, (v) => `${Math.round(cutoffHz(v))} Hz`), f('Resonance', 0, 1, 0.3, fmt.d2), f('Env Amount', -1, 1, 0, sgn), f('Osc Mix', 0, 1, 0.5, fmt.d2),
          f('Attack', 0, 4, 0.02, sec), f('Release', 0, 4, 0.3, sec), f('LFO Depth', 0, 1, 0, fmt.d2), f('LFO Rate', 0.05, 15, 4, (v) => `${v.toFixed(1)} Hz`, { log: true }),
          ctl.choice('Osc 1 Wave', WAVE_NAMES, 0, { onChange: sync }), ctl.choice('Osc 2 Wave', WAVE_NAMES, 0, { onChange: sync }),
          f('Detune', -12, 12, 0.15, (v) => `${sgn(v)} st`), f('Sub', 0, 1, 0.3, fmt.d2), f('Noise', 0, 1, 0, fmt.d2),
          ctl.int('Unison', 1, 4, 1, { onChange: sync }), ctl.int('Octave', -4, 4, 0, { fmt: (v) => `${v >= 0 ? '+' : ''}${v}`, onChange: sync }),
          ctl.int('Preset', 0, 19, 0, { fmt: (v) => `${pre(v)} ${PRESETS[v].name}`, onChange: (v) => { if (!inst.loading) load(inst, v); } }),
        ];
        const ex = (inst.ex = {}), extra = [];
        const put = (key, ctrl) => { ex[key] = ctrl; extra.push(ctrl); };
        const push = (key, v) => { p[key] = typeof v === 'boolean' ? v : v; update(inst); };
        const mf = (label, key, min, max, def, fm, o = {}) => put(key, ctl.f(label, min, max, def, { fmt: fm, onChange: (v) => push(key, v), ...o }));
        const mb = (label, key) => put(key, ctl.bool(label, 0, { onChange: (v) => push(key, !!v) }));
        const mc = (label, key, opts) => put(key, ctl.choice(label, opts, 0, { onChange: (v) => push(key, v) }));
        mf('Unison Det', 'unison_detune', 0, 1, 0.15, fmt.d2); mb('Mono', 'mono'); mf('Glide', 'glide', 0, 1, 0, (v) => fmt.ms(v * v * 1000));
        mf('F Attack', 'filter_attack', 0, 4, 0, sec); mf('F Decay', 'filter_decay', 0, 4, 0.4, sec); mf('F Sustain', 'filter_sustain', 0, 1, 0.3, fmt.d2); mf('F Release', 'filter_release', 0, 4, 0.25, sec);
        mf('Decay', 'amp_decay', 0, 4, 0.3, sec); mf('Sustain', 'amp_sustain', 0, 1, 0.8, fmt.d2);
        mc('LFO Dest', 'lfo_dest', ['Pitch', 'Filter']); mf('Chorus', 'chorus', 0, 1, 0, fmt.d2);
        mf('Delay Time', 'delay_time', 0.05, 1, 0.375, (v) => `${Math.round(v * 1000)} ms`); mf('Delay Fdbk', 'delay_feedback', 0, 0.9, 0.35, fmt.d2); mf('Delay Mix', 'delay_mix', 0, 1, 0, fmt.d2);
        mf('Reverb Size', 'reverb_size', 0, 1, 0.5, fmt.d2); mf('Reverb Mix', 'reverb_mix', 0, 1, 0, fmt.d2);
        mb('Arp', 'arp_on'); mc('Arp Pattern', 'arp_pattern', ['Up', 'Down', 'Up/Down', 'Random']); mf('Arp Rate', 'arp_rate', 1, 16, 8, (v) => `${v.toFixed(1)} Hz`);
        inst.c = c; inst.extra = extra;
        setTimeout(() => load(inst, 0), 0);
        return { controls: c, menu: [...c, ...extra], hero: [[0, 1], [2, 3], [4, 5], [6, 7]], layers: ['KEYS'], browse: 14, routes: { sx: 0, sy: 6, hl: 1, hr: 3 } };
      },
      line: (inst) => { const held = [...inst.voices.keys()].slice(0, 4).map((p) => R.noteName(R.padNote(p, { base: 48, transpose: inst.p.octave * 12 }))); return held.length ? held.join(' ') : title(inst); },
      padLabel: (inst, p) => R.noteName(R.padNote(p, { base: 48, transpose: inst.p.octave * 12 })),
      padState: (inst, p) => (R.padNote(p, { base: 48 }) % 12 === 0 ? 1 : 0),
      drawRight: drawPanels,
      noteOn(inst, pad, vel, t) { return notePress(inst, pad, vel, t); },
      transport: (inst) => ({ label: () => (inst.p.arp_on ? 'ARP OFF' : 'ARP ON'), running: () => !!inst.p.arp_on, toggle: () => { const a = inst.ex.arp_on; a.base = inst.p.arp_on ? 0 : 1; a.onChange(a.base); } }),
      enter(inst) { ensureFx(inst); },
      leave(inst) { stopArp(inst); },
    }),
  });

  const title = (inst) => `${pre(inst.c[15].base)} ${PRESETS[inst.c[15].base].name}`;
  function load(inst, i) {
    const p = PRESETS[i]; inst.loading = true; Object.assign(inst.p, p);
    const c = inst.c, e = inst.extra, set = (ctrl, v) => { ctrl.base = v; };
    set(c[0], p.filter_cutoff); set(c[1], p.filter_resonance); set(c[2], p.filter_env_amount); set(c[3], p.osc_mix); set(c[4], p.amp_attack); set(c[5], p.amp_release);
    set(c[6], p.lfo_depth); set(c[7], p.lfo_rate); set(c[8], p.osc1_wave); set(c[9], p.osc2_wave); set(c[10], p.osc2_detune); set(c[11], p.sub_level); set(c[12], p.noise_level);
    set(c[13], p.unison); set(c[14], p.octave);
    Object.keys(inst.ex).forEach((k) => { const v = p[k]; inst.ex[k].base = typeof v === 'boolean' ? (v ? 1 : 0) : v; });
    c[15].base = i; inst.loading = false; update(inst);
  }

  /* ---------------- audio ---------------- */
  function ensureFx(inst) {
    if (inst.fx || !A.ctx) return;
    const c = A.ctx, fx = (inst.fx = {});
    fx.in = c.createGain(); fx.dry = c.createGain(); fx.out = c.createGain();
    fx.in.connect(fx.dry); fx.dry.connect(fx.out);
    // chorus: two modulated short delays
    fx.chorus = c.createGain(); fx.chorus.gain.value = 0;
    [0.018, 0.027].forEach((d, i) => {
      const dl = c.createDelay(0.1); dl.delayTime.value = d;
      const l = c.createOscillator(); l.frequency.value = 0.6 + i * 0.35; const lg = c.createGain(); lg.gain.value = 0.004; l.connect(lg); lg.connect(dl.delayTime); l.start();
      fx.in.connect(dl); dl.connect(fx.chorus);
    });
    fx.chorus.connect(fx.out);
    // echo
    fx.delay = c.createDelay(1.2); fx.fb = c.createGain(); fx.dmix = c.createGain(); fx.dmix.gain.value = 0;
    const lp = c.createBiquadFilter(); lp.type = 'lowpass'; lp.frequency.value = 3600;
    fx.in.connect(fx.delay); fx.delay.connect(lp); lp.connect(fx.fb); fx.fb.connect(fx.delay); lp.connect(fx.dmix); fx.dmix.connect(fx.out);
    fx.out.connect(inst.ch.input);
    update(inst);
  }
  function update(inst) {
    const p = inst.p; if (!A.ctx) return;
    ensureFx(inst); const fx = inst.fx, t = A.T();
    fx.chorus.gain.setTargetAtTime(p.chorus * 0.7, t, 0.05);
    fx.delay.delayTime.setTargetAtTime(p.delay_time, t, 0.05); fx.fb.gain.setTargetAtTime(p.delay_feedback, t, 0.05); fx.dmix.gain.setTargetAtTime(p.delay_mix, t, 0.05);
    inst.ch.setSend(0.05 + p.reverb_mix * 0.5);
    inst.voices.forEach((v) => v && v.update && v.update(p));
    if (inst.mono && inst.mono.voice) inst.mono.voice.update(p);
    if (p.arp_on) startArp(inst); else stopArp(inst);
  }

  function makeVoice(inst, t, midi, vel) {
    const p = inst.p, c = A.ctx, f = A.midiHz(midi + p.octave * 12);
    const sources = [], mix = c.createGain(), uni = Math.max(1, p.unison);
    const norm = 1 / (Math.sqrt(uni) * (1 + (p.osc_mix > 0 ? 1 : 0) + p.sub_level));
    const oscs = [];
    const add = (type, freq, lvl, det) => { const o = A.osc(type, freq, t); o.detune.value = det; const g = A.gain(lvl); o.connect(g); g.connect(mix); o.start(t); sources.push(o); oscs.push(o); };
    for (let u = 0; u < uni; u++) {
      const spread = uni === 1 ? 0 : ((u / (uni - 1)) * 2 - 1) * p.unison_detune * 40;
      add(WAVES[p.osc1_wave], f, norm * (1 - p.osc_mix) * 1.4, spread);
      add(WAVES[p.osc2_wave], f * 2 ** (p.osc2_detune / 12), norm * p.osc_mix * 1.4, spread);
    }
    if (p.sub_level) add('square', f / 2, norm * p.sub_level * 0.8, 0);
    if (p.noise_level) { const n = A.noiseSrc(); const g = A.gain(p.noise_level * 0.3); n.connect(g); g.connect(mix); n.start(t); sources.push(n); }
    const lp = A.filt('lowpass', 1000, 0.5 + p.filter_resonance * 14);
    const base = cutoffHz(p.filter_cutoff), top = clamp(base * 2 ** (p.filter_env_amount * 4.1), 30, 18000), sus = clamp(base * 2 ** (p.filter_env_amount * 4.1 * p.filter_sustain), 30, 18000);
    lp.frequency.setValueAtTime(p.filter_attack > 0.003 ? base : top, t);
    if (p.filter_attack > 0.003) lp.frequency.exponentialRampToValueAtTime(top, t + p.filter_attack);
    lp.frequency.setTargetAtTime(sus, t + Math.max(0.001, p.filter_attack), Math.max(0.01, p.filter_decay / 3));
    const lfo = A.osc('sine', p.lfo_rate, t), lg = A.gain(0); lfo.connect(lg); lfo.start(t); sources.push(lfo);
    const aim = () => { const pp = inst.p; lg.disconnect(); lg.gain.value = pp.lfo_dest ? pp.lfo_depth * 1400 : pp.lfo_depth * 60; if (pp.lfo_dest) lg.connect(lp.detune); else oscs.forEach((o) => lg.connect(o.detune)); };
    aim();
    const env = A.gain(0), atk = Math.max(0.003, p.amp_attack), peak = 0.55 * vel;
    env.gain.setValueAtTime(0, t); env.gain.linearRampToValueAtTime(peak, t + atk);
    env.gain.setTargetAtTime(peak * p.amp_sustain, t + atk, Math.max(0.01, p.amp_decay / 3));
    const rel = A.gain(1);
    mix.connect(lp); lp.connect(env); env.connect(rel); rel.connect(inst.fx.in);
    sources.forEach((s) => { try { s.stop(t + 60); } catch (_) { /* ignore */ } });
    return {
      oscs, lp,
      glideTo(m, glide) { const fr = A.midiHz(m + p.octave * 12); oscs.forEach((o, i) => { const mult = o === oscs[oscs.length - 1] && p.sub_level ? 0.5 : (i % 2 ? 2 ** (p.osc2_detune / 12) : 1); o.frequency.setTargetAtTime(fr * mult, A.T(), Math.max(0.005, glide * glide / 3)); }); },
      update(pp) { const b = cutoffHz(pp.filter_cutoff); lp.frequency.cancelScheduledValues(A.T()); lp.frequency.setTargetAtTime(clamp(b * 2 ** (pp.filter_env_amount * 4.1 * pp.filter_sustain), 30, 18000), A.T(), 0.03); lp.Q.setTargetAtTime(0.5 + pp.filter_resonance * 14, A.T(), 0.03); lfo.frequency.setTargetAtTime(pp.lfo_rate, A.T(), 0.03); aim(); },
      stop(t2) {
        const r = Math.max(0.03, p.amp_release);
        lp.frequency.cancelScheduledValues(t2); lp.frequency.setTargetAtTime(base, t2, Math.max(0.01, p.filter_release / 3));
        A.release(rel, sources, t2, r);
      },
    };
  }

  function notePress(inst, pad, vel, t) {
    ensureFx(inst);
    const midi = R.padNote(pad, { base: 48 }), p = inst.p;
    if (p.arp_on) {
      inst.arpHeld = inst.arpHeld || []; inst.arpHeld.push(midi); startArp(inst);
      return { stop() { inst.arpHeld = (inst.arpHeld || []).filter((m) => m !== midi); } };
    }
    if (p.mono) {
      const m = (inst.mono = inst.mono || { voice: null, stack: [] }); m.stack.push(midi);
      if (m.voice) m.voice.glideTo(midi, p.glide); else m.voice = makeVoice(inst, t, midi, vel);
      return { stop(t2) { m.stack = m.stack.filter((x) => x !== midi); if (!m.stack.length) { m.voice.stop(t2); m.voice = null; } else m.voice.glideTo(m.stack[m.stack.length - 1], p.glide); } };
    }
    return makeVoice(inst, t, midi, vel);
  }
  function startArp(inst) {
    if (inst.arp && inst.arp.on) return;
    if (!A.ctx) return;
    let i = 0, dir = 1;
    inst.arp = new A.Scheduler(() => 1 / inst.p.arp_rate, (s, t) => {
      const held = (inst.arpHeld || []).slice().sort((a, b) => a - b); if (!held.length) return;
      const pat = inst.p.arp_pattern; let idx;
      if (pat === 0) idx = i++ % held.length; else if (pat === 1) idx = held.length - 1 - (i++ % held.length);
      else if (pat === 2) { idx = clamp(i, 0, held.length - 1); i += dir; if (i >= held.length - 1) dir = -1; if (i <= 0) dir = 1; } else idx = Math.floor(Math.random() * held.length);
      const v = makeVoice(inst, t, held[idx], 0.85); v.stop(t + 0.7 / inst.p.arp_rate);
    });
    inst.arp.start();
  }
  function stopArp(inst) { if (inst.arp) { inst.arp.stop(); inst.arp = null; } }

  /* ---------------- the four panels ---------------- */
  const { rgba } = R;
  function waveSample(w, ph) { return w === 0 ? 2 * ph - 1 : w === 1 ? (ph < 0.5 ? 1 : -1) : w === 2 ? 4 * Math.abs(ph - 0.5) - 1 : Math.sin(ph * 2 * Math.PI); }
  function trace(fb, pts, color) { const g = fb.g; g.strokeStyle = color; g.lineWidth = 2; g.lineJoin = 'round'; g.beginPath(); pts.forEach(([x, y], i) => (i ? g.lineTo(x, y) : g.moveTo(x, y))); g.stroke(); }
  function sub(fb, pal, x, y, w, h, name) { fb.rr(x, y, w, h, 3, rgba(pal.ink, 0.025), rgba(pal.ink, 0.16)); fb.ttf(name, x + 4, y + 14, 12, pal.accent, { family: 'sans' }); return [x + 4, y + 20, w - 8, h - 24]; }
  function drawPanels(fb, inst, pal) {
    const p = inst.p; R.slint.card(fb, pal, '', 316, 45, 306, 260);
    fb.rect(324, 53, 3, 13, pal.accent); fb.ttf(title(inst), 334, 64, 12, rgba(pal.ink, 0.85), { spacing: 1 });
    const gw = 143, gh = 105, X0 = 324, Y0 = 74;
    let [x, y, w, h] = sub(fb, pal, X0, Y0, gw, gh, 'Oscillators'), pts = [];
    for (let i = 0; i <= w; i++) { const t = i / w, s1 = waveSample(p.osc1_wave, (t * 2) % 1), s2 = waveSample(p.osc2_wave, (t * 2) % 1); const v = clamp(((s1 * (1 - p.osc_mix) + s2 * p.osc_mix) + waveSample(1, t) * p.sub_level) / (1 + p.sub_level), -1.5, 1.5); pts.push([x + i, y + h / 2 - v * (h / 2 - 3)]); }
    trace(fb, pts, pal.accent);
    [x, y, w, h] = sub(fb, pal, X0 + gw + 8, Y0, gw, gh, 'Filter'); pts = [];
    const cut = p.filter_cutoff, res = p.filter_resonance;
    for (let i = 0; i <= w; i++) { const fr = i / w, dist = (fr - cut) * 8, peak = (1 + res * 3) * Math.exp(-dist * dist), base = fr < cut ? 1 : Math.exp(-(fr - cut) * 4); pts.push([x + i, y + h - Math.min(base + peak, 2.2) / 2.2 * h]); }
    trace(fb, pts, pal.accent);
    fb.rect(x + clamp(cut, 0, 1) * w, y, 1.5, h, '#ff5a4a');
    if (Math.abs(p.filter_env_amount) > 0.01) fb.rect(x + clamp(cut + p.filter_env_amount * 0.5, 0, 1) * w, y, 1.5, h, '#f0b030');
    [x, y, w, h] = sub(fb, pal, X0, Y0 + gh + 8, gw, gh, 'Amp Envelope');
    const a = Math.max(0.03, p.amp_attack), d = Math.max(0.03, p.amp_decay), s = clamp(p.amp_sustain, 0, 1), r = Math.max(0.03, p.amp_release), sc = 0.7 / (a + d + r);
    const ax = x + a * sc * w, dx = ax + d * sc * w, sx = dx + 0.3 * w, ly = (l) => y + h - l * h;
    trace(fb, [[x, y + h], [ax, ly(1)], [dx, ly(s)], [sx, ly(s)], [x + w, y + h]], pal.accent);
    [x, y, w, h] = sub(fb, pal, X0 + gw + 8, Y0 + gh + 8, gw, gh, 'LFO'); pts = [];
    const cycles = 1 + ((p.lfo_rate - 0.05) / 14.95) * 5;
    for (let i = 0; i <= w; i++) pts.push([x + i, y + h / 2 - Math.sin((i / w) * cycles * 2 * Math.PI) * p.lfo_depth * (h / 2 - 3)]);
    trace(fb, pts, pal.accent);
  }
})();
