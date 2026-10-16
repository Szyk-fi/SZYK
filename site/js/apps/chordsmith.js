/* Chordsmith: play full, voice-led chords from the pads. The bottom two rows are the chords of the key; hold
   pads in the top two rows to change them (7th, sus4, add9, flip, 6th, power, 5 bass, wide). The chord
   theory is ported from the app: the same degrees, names and voice-leading search. */
(() => {
  'use strict';
  const R = window.ROVE, A = R.audio, { c565, blend } = R;
  const NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
  const MAJOR = [0, 2, 4, 5, 7, 9, 11], MINOR = [0, 2, 3, 5, 7, 8, 10];
  const MODS = ['7th', 'sus4', 'add9', 'flip', '6th', 'power', '5 bass', 'wide'];
  const STYLES = ['Pad', 'Strum', 'Arp up', 'Arp up-down', 'Pulse'], TONES = ['Harp', 'Soft', 'Organ', 'Marimba', 'Glock'];
  const NUM_MAJ = ['I', 'ii', 'iii', 'IV', 'V', 'vi', 'vii°', 'bVII'], NUM_MIN = ['i', 'ii°', 'III', 'iv', 'v', 'VI', 'VII', 'bVII'];
  const LOW = 52, HIGH = 79;
  const sn = (scale, i) => scale[((i % 7) + 7) % 7] + 12 * Math.floor(i / 7);
  const FLATS = ['C', 'Db', 'D', 'Eb', 'E', 'F', 'F#', 'G', 'Ab', 'A', 'Bb', 'B'];
  const pcName = (n) => FLATS[((n % 12) + 12) % 12];
  const padDegree = (p) => (p >= 8 ? (p >= 12 ? p - 12 : p - 4) : null);

  function build(key, minor, degree, mods) {
    const scale = minor ? MINOR : MAJOR; let root, third, fifth;
    if (degree === 7) { root = key + 10; third = 4; fifth = 7; } else { const r = sn(scale, degree); root = key + r; third = sn(scale, degree + 2) - r; fifth = sn(scale, degree + 4) - r; }
    if (mods[3]) { third = third === 4 ? 3 : 4; if (fifth === 6) fifth = 7; }
    const iv = [0];
    if (mods[5]) iv.push(7);
    else {
      iv.push(mods[1] ? 5 : third); iv.push(fifth);
      if (mods[0]) iv.push((degree === 7 ? 10 : sn(scale, degree + 6) - sn(scale, degree) + 120) % 12);
      if (mods[4]) iv.push(9);
      if (mods[2]) iv.push(14);
    }
    return [((root % 12) + 12) % 12, iv];
  }
  function name(root, iv, mods) {
    const r = pcName(root); if (mods[5]) return `${r}5`;
    const third = iv[1] ?? 4, fifth = iv[2] ?? 7; let s = third === 5 ? `${r}sus4` : third === 3 && fifth === 6 ? `${r}dim` : third === 3 ? `${r}m` : r;
    const sev = iv.find((i) => i === 10 || i === 11);
    if (sev != null) { s += sev === 11 ? 'maj7' : '7'; if (third === 3 && sev === 11) s = s.replace('mmaj7', 'm(maj7)'); }
    if (iv.includes(9)) s += '6'; if (iv.includes(14)) s += '(add9)'; return s;
  }
  const movement = (v, prev) => v.reduce((a, x) => a + Math.min(...prev.map((b) => Math.abs(x - b))), 0) + Math.abs(v.length - prev.length) * 2;
  const spread = (v, wide) => { if (wide && v.length >= 3) { v[1] += 12; v.sort((a, b) => a - b); } return v; };
  function voice(pcs, prev, wide) {
    const n = pcs.length;
    if (!prev.length) {
      const d = (((pcs[0] - 60) % 12) + 12) % 12, base = 60 + d - (d > 5 ? 12 : 0), v = []; let last = base - 1;
      pcs.forEach((pc) => { let x = last + 1 + ((((pc - (last + 1)) % 12) + 12) % 12); if (!v.length) x = base; v.push(x); last = x; });
      return spread(v, wide);
    }
    let best = null;
    for (let rot = 0; rot < n; rot++) {
      const order = Array.from({ length: n }, (_, k) => pcs[(rot + k) % n]);
      for (let start = LOW; start <= HIGH; start++) {
        if (((start % 12) + 12) % 12 !== ((order[0] % 12) + 12) % 12) continue;
        const v = [start]; order.slice(1).forEach((pc) => { const last = v[v.length - 1]; v.push(last + 1 + ((((pc - (last + 1)) % 12) + 12) % 12)); });
        if (v[v.length - 1] > HIGH + 4) continue;
        const cost = movement(v, prev); if (!best || cost < best[0]) best = [cost, v];
      }
    }
    return spread(best ? best[1] : [], wide);
  }

  R.registerApp({
    id: 'chordsmith', name: 'Chordsmith', category: 'instrument',
    desc: 'Play full, voice-led chords from the pads: the chords of a key on the bottom rows, modifiers on the top.',
    create(os) {
      const ch = A.channel('Chordsmith');
      const inst = { view: 'full', ch, key: 0, minor: false, row: 0, style: 1, tone: 0, strum: 22, tempo: 100, bass: 1, mods: Array(8).fill(false), prevNotes: [], cur: null, history: [], held: new Set(), voices: [], flash: 0 };
      const ROWS = ['Plays', 'Sound', 'Key', 'Mode', 'Style', 'Strum', 'Tempo', 'Bass'];
      const value = (r) => ({ Plays: 'Own sound', Sound: TONES[inst.tone], Key: pcName(inst.key), Mode: inst.minor ? 'minor' : 'major', Style: STYLES[inst.style], Strum: `${inst.strum} ms`, Tempo: `${inst.tempo} bpm`, Bass: ['off', 'low', 'lower'][inst.bass] }[r]);
      const change = (d) => {
        const r = ROWS[inst.row];
        if (r === 'Sound') inst.tone = (inst.tone + d + 5) % 5; else if (r === 'Key') inst.key = (inst.key + d + 12) % 12; else if (r === 'Mode') inst.minor = !inst.minor;
        else if (r === 'Style') inst.style = (inst.style + d + 5) % 5; else if (r === 'Strum') inst.strum = Math.max(4, Math.min(120, inst.strum + d * 4)); else if (r === 'Tempo') inst.tempo = Math.max(40, Math.min(220, inst.tempo + d * 2));
        else if (r === 'Bass') inst.bass = (inst.bass + d + 3) % 3;
      };
      const numeral = (d) => (inst.minor ? NUM_MIN : NUM_MAJ)[d];
      const ensure = () => { if (!inst.bus && A.ctx) { inst.bus = A.gain(1); inst.bus.connect(inst.ch.input); inst.ch.setSend(0.28); } };

      function strike(notes, t, vel, hold) {
        const tone = inst.tone, v = [];
        notes.forEach((m, i) => {
          const f = A.midiHz(m), tt = t + (inst.style === 1 ? i * inst.strum / 1000 : 0);
          if (tone === 0) v.push(A.pluckVoice(inst.bus, tt, f, vel * 0.8, { damp: 0.3, t60: 2.4, level: 0.8 }));
          else if (tone === 1) v.push(A.subVoice(inst.bus, tt, f, vel, { wave: 'sawtooth', mix2: 0.8, detune: 12, cutoff: 1400, res: 0.6, atk: 0.25, dec: 0.5, sus: 0.8, rel: 0.6, level: 0.16 }));
          else if (tone === 2) v.push(A.subVoice(inst.bus, tt, f, vel, { wave: 'square', mix2: 0.5, wave2: 'sine', detune: 0, cutoff: 3500, res: 0.4, atk: 0.01, dec: 0.1, sus: 1, rel: 0.08, level: 0.18 }));
          else if (tone === 3) v.push(A.fmVoice(inst.bus, tt, f, vel, { rc: 1, r1: 4, r2: 10, i1: 0.5, i2: 0.2, decay: 0.7, atk: 0.002, level: 0.5 }));
          else v.push(A.fmVoice(inst.bus, tt, f, vel, { rc: 1, r1: 3.5, r2: 7, i1: 1.1, i2: 0.4, decay: 1.8, atk: 0.002, level: 0.4 }));
        });
        return v;
      }
      function play(d) {
        ensure(); const [root, iv] = build(inst.key, inst.minor, d, inst.mods), pcs = iv.map((i) => root + i);
        const notes = voice(pcs, inst.prevNotes, inst.mods[7]); inst.prevNotes = notes;
        const label = name(root, iv, inst.mods); inst.cur = { d, notes, label, bass: inst.bass ? 24 + root + (inst.bass === 2 ? 0 : 12) : null };
        inst.history.unshift(label); inst.history = inst.history.slice(0, 8);
        const all = inst.cur.bass != null ? [inst.cur.bass + 12, ...notes] : notes;
        stopVoices(); const t = A.T() + 0.003;
        if (inst.style === 0 || inst.style === 1) inst.voices = strike(all, t, 0.85, true);
        else {
          let k = 0, dir = 1; inst.voices = [];
          inst.arp = new A.Scheduler(() => 60 / inst.tempo / (inst.style === 4 ? 2 : 4), (s, tt) => {
            if (inst.style === 4) { strike(notes, tt, 0.6); return; }
            const seq = notes; let i; if (inst.style === 2) i = k++ % seq.length; else { i = Math.max(0, Math.min(seq.length - 1, k)); k += dir; if (k >= seq.length - 1) dir = -1; if (k <= 0) dir = 1; }
            strike([seq[i]], tt, 0.7);
          }); inst.arp.start();
          if (inst.cur.bass != null) inst.voices = strike([inst.cur.bass + 12], t, 0.8);
        }
      }
      function stopVoices() { if (inst.arp) { inst.arp.stop(); inst.arp = null; } inst.voices.forEach((v) => v && v.stop && v.stop(A.T())); inst.voices = []; }

      inst.tick = (inp) => {
        if (inp.navY) inst.row = Math.max(0, Math.min(ROWS.length - 1, inst.row + Math.sign(inp.navY)));
        if (inp.navX) change(Math.sign(inp.navX));
        for (let p = 0; p < 8; p++) inst.mods[p] = inp.held[p];
        inp.down.forEach((d, p) => { const deg = padDegree(p); if (d && deg != null && A.ctx) { inst.degHeld = p; play(deg); } });
        inp.up.forEach((u, p) => { if (u && inst.degHeld === p) { stopVoices(); inst.degHeld = null; } });
      };
      inst.draw = (fb) => {
        const bg = c565(4, 4, 7), panel = c565(8, 8, 14), dim = c565(16, 26, 24), ink = '#F0C878', W = '#FFFFFF';
        fb.clear(bg); fb.rect(0, 0, 640, 30, panel);
        fb.ktext('Chordsmith', 12, 7, 'medium', W); fb.ktext(`${pcName(inst.key)} ${inst.minor ? 'minor' : 'major'}`, 630, 9, 'small', dim, 1);
        ROWS.forEach((r, i) => {
          const y = 38 + i * 28, sel = i === inst.row; fb.roundRect(8, y, 200, 25, 6, sel ? blend(panel, W, 0.15) : panel);
          fb.ktext(r, 16, y + 6, 'small', sel ? W : dim); fb.ktext(value(r), 200, y + 6, 'small', ink, 1);
        });
        fb.roundRect(218, 38, 414, 110, 12, panel);
        if (inst.cur) { fb.ktext(inst.cur.label, 330, 54, 'huge', W, 0); fb.ktext(numeral(inst.cur.d), 540, 64, 'large', ink, 0); } else fb.ktext('-', 330, 54, 'huge', dim, 0);
        const held = new Set(inst.degHeld != null && inst.cur ? [...inst.cur.notes, ...(inst.cur.bass != null ? [inst.cur.bass + 12] : [])] : []);
        const whites = []; for (let n = 36; n <= 84; n++) if (![1, 3, 6, 8, 10].includes(n % 12)) whites.push(n);
        const w = Math.floor(398 / whites.length), kx = 226, ky = 156;
        whites.forEach((n, k) => fb.rect(kx + k * w, ky, w - 1, 52, held.has(n) ? ink : '#DCDCE1'));
        whites.forEach((n, k) => { if ([0, 2, 5, 7, 9].includes(n % 12) && n < 84) fb.rect(kx + k * w + Math.floor(w * 2 / 3), ky, Math.floor(w * 2 / 3), 32, held.has(n + 1) ? '#C88C3C' : '#1E1E24'); });
        for (let p = 0; p < 16; p++) {
          const x = 226 + (p % 4) * 100, y = 218 + Math.floor(p / 4) * 24, d = padDegree(p); let label, c;
          if (d != null) { const [r, iv] = build(inst.key, inst.minor, d, Array(8).fill(false)); label = `${numeral(d)} ${name(r, iv, Array(8).fill(false))}`; c = inst.cur && inst.cur.d === d && inst.degHeld != null ? ink : blend(panel, W, 0.12); }
          else { label = MODS[p]; c = inst.mods[p] ? '#78AAF0' : panel; }
          fb.roundRect(x, y, 96, 21, 5, c); fb.ktext(label, x + 48, y + 4, 'small', W, 0);
        }
        fb.ktext(inst.history.join('  '), 12, 286, 'small', dim);
        fb.rect(0, 338, 640, 22, panel); fb.ktext('Bottom rows: chords   top rows: hold to change them   up/down: setting   left/right: change', 12, 343, 'small', dim);
      };
      inst.leave = () => stopVoices();
      inst.ledStates = () => Array.from({ length: 16 }, (_, p) => { const d = padDegree(p); return d != null ? (inst.cur && inst.cur.d === d && inst.degHeld != null ? 4 : [0, 2, 5].includes(d) ? 3 : [1, 3].includes(d) ? 1 : 2) : (inst.mods[p] ? 4 : 0); });
      inst.speak = () => `Chordsmith. Key ${pcName(inst.key)} ${inst.minor ? 'minor' : 'major'}. ${inst.cur ? `Chord ${inst.cur.label}.` : ''}`;
      return inst;
    },
  });
})();
