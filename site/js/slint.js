/* The Slint shell's look, in canvas: the title row, the shared play column, the right-hand
   instrument cards, the hint line and the F-key bar. Geometry is measured from the shell's
   own renders (examples/slint_common/*.slint), so apps line up with the real screens. */
(() => {
  'use strict';
  const R = window.ROVE;
  const { rgba } = R;
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const S = {};

  S.pal = (bg, ink, accent, dim) => ({ bg, ink, accent, dim: dim || rgba(ink, 0.55), paper: bg });

  /* title row: app name in the accent, breadcrumb on the right */
  S.chrome = (fb, pal, title, crumb) => {
    fb.clear(pal.bg);
    fb.ttf(title, 18, 33, 27, pal.accent, { family: 'sans', weight: 700, spacing: 0.4 });
    fb.ttf(crumb, 622, 35, 12, rgba(pal.ink, 0.72), { align: 'right', spacing: 0.6 });
  };

  S.hint = (fb, pal, text) => fb.ttf(text.replace(/◀▶/g, '◄►'), 18, 319, 11, rgba(pal.ink, 0.62), { family: 'sans', maxW: 604 });

  /* F1-F4 bar: cells share the width in proportion to their text, like the shell's layout */
  S.bar = (fb, pal, labels, f3Lit) => {
    fb.rect(0, 324, 640, 1, rgba(pal.ink, 0.14));
    const texts = labels.map((l, i) => `F${i + 1}  ${l || '—'}`);
    const ws = texts.map((t, i) => fb.ttfW(t, 14, { spacing: 0.4, weight: i === 2 && f3Lit ? 700 : 400 }));
    const extra = (640 - ws.reduce((a, b) => a + b, 0)) / 4;
    let x = 0;
    texts.forEach((t, i) => {
      const lit = i === 2 && f3Lit;
      fb.ttf(t, x, 347, 14, lit ? pal.accent : rgba(pal.accent, 0.72), { spacing: 0.4, weight: lit ? 700 : 400 });
      x += ws[i] + extra;
    });
  };

  /* ---------------- the play column ---------------- */
  function dial(fb, pal, x, y, d) {
    const n = Math.max(0.002, Math.min(1, d.norm)), cx = x + 17, cy = y + 17, a0 = (135 * Math.PI) / 180;
    fb.arc(cx, cy, 13, a0, a0 + (270 * Math.PI) / 180, rgba(pal.ink, 0.14), 4);
    fb.arc(cx, cy, 13, a0, a0 + n * (270 * Math.PI) / 180, d.knob === 2 ? pal.accent : d.knob === 1 ? rgba(pal.accent, 0.55) : rgba(pal.ink, 0.45), 4);
    if (d.knob === 2) fb.disc(cx - 6, cy - 6, 12, pal.accent);
    fb.ttf(d.label, x + 39, y + 14, 12, pal.ink, { family: 'sans', weight: 700, maxW: d.tw || 98 });
    fb.ttf(d.value, x + 39, y + 28, 12, rgba(pal.ink, 0.72), { maxW: d.tw || 96 });
  }

  S.column = (fb, kit, pal, o = {}) => {
    const x = 18, y = 44, w = o.w || 280;
    fb.rr(x, y, 128, 16, 3, pal.accent);
    fb.ttf(`PADS · ${kit.layer.label}`, x + 64, y + 12, 12, pal.paper, { weight: 700, align: 'center' });
    if (w >= 270) fb.ttf('F2 LAYER · R1 MENU', x + w, y + 12, 12, rgba(pal.ink, 0.72), { align: 'right' });

    const pages = kit.hero.length, first = pages ? (Math.floor(kit.focus / 2) % pages) & ~1 : 0, ids = [];
    for (let p = first; p < Math.min(first + 2, pages); p++) ids.push(...kit.hero[p]);
    const fc = kit.flat[kit.focus % kit.flat.length], pr = kit.pair(), cw = (w - 6) / 2;
    kit.touchActions = [{x,y,w:128,h:20,kind:'layer'},{x:x+134,y,w:w-134,h:20,kind:'menu'}];
    kit.touchPads = [];
    kit.touchDials = [];
    ids.slice(0, 4).forEach((cid, i) => {
      kit.touchDials.push({ cid, x: x + (i % 2) * (cw + 6), y: y + 21 + Math.floor(i / 2) * 39, w: cw, h: 36 });
      const c = kit.controls[cid];
      dial(fb, pal, x + (i % 2) * (cw + 6), y + 21 + Math.floor(i / 2) * 39, { label: c.label, value: c.text, norm: c.effNorm, knob: fc === cid ? 2 : pr.includes(cid) ? 1 : 0, tw: cw - 44 });
    });
    let py = y + 99;
    if (!ids.includes(fc)) { fb.ttf(`→ ${kit.controls[fc].label}  ${kit.controls[fc].text}`, x, py + 11, 12, pal.accent, { weight: 700, maxW: w }); py += 20; }

    const pw = (w - 9) / 4;
    for (let i = 0; i < 16; i++) {
      const px = x + (i % 4) * (pw + 3), pyy = py + Math.floor(i / 4) * 27, st = o.padState ? o.padState(i) : 0;
      kit.touchPads.push({i,x:px,y:pyy,w:pw,h:24});
      const fill = st >= 3 ? pal.accent : st === 1 ? rgba(pal.accent, 0.22) : st === 2 ? 'rgba(224,160,48,0.28)' : rgba(pal.ink, 0.03);
      fb.rr(px, pyy, pw, 24, 4, fill, st === 4 ? pal.ink : rgba(pal.ink, 0.18));
      fb.ttf(o.padLabel ? o.padLabel(i) : '', px + 4, pyy + 16, 12, st >= 3 ? pal.paper : pal.ink, { maxW: pw - 8 });
    }
    py += 4 * 27 + 5 - 3 + 0;
    // stick and hands
    kit.touchStick = {x,y:py,w:44,h:44,kind:"stick"};
    kit.touchHands = [];
    fb.rr(x, py, 44, 44, 3, null, rgba(pal.ink, 0.2));
    fb.rect(x + 22, py, 1, 44, rgba(pal.ink, 0.1)); fb.rect(x, py + 22, 44, 1, rgba(pal.ink, 0.1));
    fb.disc(x + 22 + clamp(kit.stick[0], -1, 1) * 17 - 4, py + 22 - clamp(kit.stick[1], -1, 1) * 17 - 4, 8, pal.accent);
    const sl = `${kit.routeLabel(kit.routes.sx)} / ${kit.routeLabel(kit.routes.sy)}`;
    fb.ttf(`STICK ${sl}`, x + 50, py + 17, 12, rgba(pal.ink, 0.72), { maxW: 130 });
    const line = kit.status || (o.line ? o.line() : 'Play');
    fb.ttf(line, x + 50, py + 34, 12, kit.status ? pal.ink : pal.accent, { weight: 700, maxW: 130 });
    [[0, x + w - 74, kit.routes.hl], [1, x + w - 36, kit.routes.hr]].forEach(([hi, hx, route]) => {
      kit.touchHands.push({x:hx,y:py,w:34,h:28,kind:"hand",i:hi});
      const v = clamp(kit.hands[hi], 0, 1);
      fb.rr(hx, py, 34, 28, 3, rgba(pal.ink, 0.08));
      if (v > 0) fb.rr(hx, py + 28 * (1 - v), 34, 28 * v, 3, pal.accent);
      fb.ttf(`${hi ? 'R' : 'L'} ${kit.routeLabel(route)}`, hx + 17, py + 42, 12, pal.ink, { align: 'center', maxW: 34 });
    });
  };

  /* the R1 menu: a parameter list, selected row filled with the accent */
  S.menu = (fb, kit, pal) => {
    kit.touchBack = {x:550,y:44,w:70,h:32};
    fb.rr(550,44,70,32,4,pal.accent);
    fb.ttf('PLAY',585,65,12,pal.paper,{align:'center'});
    const rows = kit.menuControls, n = rows.length, visible = 10, sel = kit.menuSel;
    kit.touchRows = [];
    const start = clamp(sel - 4, 0, Math.max(0, n - visible)), x = 18, w = 280;
    for (let i = start; i < Math.min(n, start + visible); i++) {
      const y = 45 + (i - start) * 23, on = i === sel;
      kit.touchRows.push({i,x,y,w,h:22});
      fb.rr(x, y, w, 22, 3, on ? pal.accent : null);
      fb.ttf(rows[i].label, x + 14, y + 15, 12, on ? pal.paper : rgba(pal.ink, 0.86), { family: 'sans', weight: on ? 600 : 400, maxW: 150 });
      fb.ttf(rows[i].text, x + w - 7, y + 15, 10, on ? pal.paper : rgba(pal.ink, 0.72), { align: 'right', maxW: 110 });
    }
    fb.ttf('R1 PLAY VIEW  ·  ◄► ADJUST', x, 292, 9, rgba(pal.ink, 0.72));
    if (kit.status) fb.ttf(kit.status, x, 305, 10, pal.accent);
  };

  /* ---------------- right-hand cards ---------------- */
  S.card = (fb, pal, header, x = 316, y = 45, w = 306, h = 260) => {
    fb.rr(x, y, w, h, 6, rgba(pal.ink, 0.035), rgba(pal.ink, 0.14));
    if (header) {
      fb.rect(x + 8, y + 8, 3, 13, pal.accent);
      fb.ttf(header, x + 17, y + 19, 12, rgba(pal.ink, 0.85), { spacing: 1 });
      fb.rect(x + 8, y + 28, w - 16, 1, rgba(pal.ink, 0.14));
    }
    return { x: x + 10, y: y + (header ? 36 : 10), w: w - 20, h: h - (header ? 46 : 20) };
  };
  S.label = (fb, pal, t, x, y, color) => fb.ttf(t, x, y, 12, color || rgba(pal.ink, 0.72), { spacing: 1 });
  S.big = (fb, pal, t, x, y, size = 21) => fb.ttf(t, x, y, size, pal.accent, { family: 'sans', weight: 700, maxW: 280 });
  S.meter = (fb, pal, x, y, w, h, v, color) => { fb.rr(x, y, w, h, h / 2, rgba(pal.ink, 0.13)); if (v > 0) fb.rr(x, y, Math.max(h, w * clamp(v, 0, 1)), h, h / 2, color || pal.accent); };
  S.scope = (fb, pal, x, y, w, h) => {
    fb.rr(x, y, w, h, 3, rgba(pal.ink, 0.025), rgba(pal.ink, 0.14));
    for (let c = 1; c < 8; c++) fb.rect(x + Math.round((w * c) / 8), y + 1, 1, h - 2, rgba(pal.ink, 0.06));
    for (let r = 1; r < 4; r++) fb.rect(x + 1, y + Math.round((h * r) / 4), w - 2, 1, rgba(pal.ink, r === 2 ? 0.18 : 0.06));
  };
  /* live waveform from the master analyser */
  const tb = new Float32Array(2048);
  S.wave = (fb, pal, x, y, w, h, color) => {
    const A = R.audio; if (!A.analyser) return;
    A.analyser.getFloatTimeDomainData(tb);
    let s0 = 0; for (let i = 1; i < 1024; i++) if (tb[i - 1] <= 0 && tb[i] > 0) { s0 = i; break; }
    let pk = 0; for (let i = 0; i < 700; i++) pk = Math.max(pk, Math.abs(tb[s0 + i]));
    const sc = pk < 0.002 ? 0 : 0.42 * h / Math.max(pk, 0.18), g = fb.g;
    g.save(); g.beginPath(); g.rect(x, y, w, h); g.clip(); g.strokeStyle = color || pal.accent; g.lineWidth = 1.6; g.beginPath();
    for (let i = 0; i < 700; i++) { const px = x + (i / 699) * w, py = y + h / 2 - tb[s0 + i] * sc; i ? g.lineTo(px, py) : g.moveTo(px, py); }
    g.stroke(); g.restore();
  };
  const sb = new Uint8Array(1024);
  S.spectrum = (fb, pal, x, y, w, h, bars = 24, color) => {
    const A = R.audio; if (!A.analyser) return;
    A.analyser.getByteFrequencyData(sb); const nyq = A.ctx.sampleRate / 2, bw = w / bars;
    for (let i = 0; i < bars; i++) {
      const f0 = 40 * (12000 / 40) ** (i / bars), f1 = 40 * (12000 / 40) ** ((i + 1) / bars);
      const b0 = Math.floor((f0 / nyq) * 1024), b1 = Math.max(b0 + 1, Math.ceil((f1 / nyq) * 1024)); let v = 0;
      for (let b = b0; b < b1; b++) v = Math.max(v, sb[b]);
      const bh = Math.max(1, (v / 255) * h); fb.rect(x + i * bw, y + h - bh, bw - 2, bh, color || rgba(pal.ink, 0.8));
    }
  };

  /* mini piano with highlighted pitch classes (scale panel and keyboards) */
  S.keys = (fb, pal, x, y, w, h, lit, labels = true) => {
    const white = [0, 2, 4, 5, 7, 9, 11], names = ['C', 'D', 'E', 'F', 'G', 'A', 'B'], kw = w / 7;
    white.forEach((p, i) => {
      const on = lit && lit[p];
      fb.rr(x + i * kw, y, kw - 2, h, 3, on ? pal.accent : rgba(pal.ink, 0.13));
      if (labels) fb.ttf(names[i], x + i * kw + (kw - 2) / 2, y + h - 6, 12, on ? pal.paper : pal.ink, { align: 'center' });
    });
    [[1, 1], [3, 2], [6, 4], [8, 5], [10, 6]].forEach(([p, pos]) => {
      const on = lit && lit[p];
      fb.rr(x + (pos - 0.3) * kw, y, kw * 0.6, h * 0.6, 2, on ? rgba(pal.accent, 0.75) : R.blend(pal.bg, '#000000', 0.6), pal.bg);
    });
  };
  S.scalePanel = (fb, pal, info) => {
    fb.ttf('MAIN SCALE', 328, 62, 12, pal.accent);
    fb.ttf(`${info.root} ${info.name}`, 328, 96, 24, pal.ink, { family: 'sans', maxW: 290 });
    fb.ttf('NOTES', 328, 158, 12, pal.accent);
    fb.ttf(info.notes, 328, 182, 14, pal.ink, { spacing: 1, maxW: 290 });
    S.keys(fb, pal, 328, 238, 290, 56, info.lit);
    fb.ttf('Highlighted keys belong to this scale', 328, 312, 12, rgba(pal.ink, 0.72), { family: 'sans' });
  };
  /* "VOICE / CONTROL" card with a big value and the hint, used by apps without a bespoke panel */
  S.paramCard = (fb, pal, header, big, sub, note = 'Up/down picks a row; left/right changes it.') => {
    S.card(fb, pal, header);
    fb.ttf(big, 324, 100, 24, pal.ink, { family: 'sans', weight: 700 });
    fb.rect(324, 110, 290, 2, pal.accent);
    fb.ttf(sub, 324, 138, 18, pal.accent, { family: 'sans', maxW: 290 });
    fb.ttf(note, 324, 296, 12, rgba(pal.ink, 0.72), { family: 'sans' });
  };

  R.slint = S;
})();
