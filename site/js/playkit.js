/* The shared "play view" every instrument-style app uses on ROVE: two pairs of dials, a 4x4 pad
   map, the stick and the two hand sensors, a status line, plus the R1 menu. This is a port of
   src/play_kit.rs (behaviour) and play_kit::draw::column (pixels). */
(() => {
  'use strict';
  const R = window.ROVE;
  const { c565 } = R;
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));

  /* ---------------------------------------------------------------- controls */
  // base is what the user set; off is a temporary offset from stick / hand / pressure.
  class Control {
    constructor(label, o) {
      this.label = label; this.kind = o.kind; this.min = o.min ?? 0; this.max = o.max ?? 1;
      this.options = o.options; this.base = o.def ?? this.min; this.def = this.base;
      this.fmt = o.fmt; this.log = !!o.log; this.stepSize = o.step; this.off = 0; this.onChange = o.onChange;
    }
    get stepped() { return this.kind !== 'f'; }
    // normalised 0..1 of the user's setting (log controls are normalised in log space)
    get norm() {
      if (this.kind === 'choice') return this.options.length > 1 ? this.base / (this.options.length - 1) : 0;
      if (this.kind === 'bool') return this.base ? 1 : 0;
      if (this.log) return Math.log(this.base / this.min) / Math.log(this.max / this.min);
      return this.max > this.min ? (this.base - this.min) / (this.max - this.min) : 0;
    }
    fromNorm(n) {
      n = clamp(n, 0, 1);
      if (this.kind === 'choice') return Math.round(n * (this.options.length - 1));
      if (this.kind === 'bool') return n >= 0.5 ? 1 : 0;
      if (this.kind === 'int') return Math.round(this.min + n * (this.max - this.min));
      return this.log ? this.min * (this.max / this.min) ** n : this.min + n * (this.max - this.min);
    }
    set norm(n) { this.base = this.fromNorm(n); if (this.onChange) this.onChange(this.value); }
    // effective value including the expression offset
    get value() { return this.stepped || !this.off ? this.base : this.fromNorm(this.norm + this.off); }
    get effNorm() { return this.stepped ? this.norm : clamp(this.norm + this.off, 0, 1); }
    nudge(steps) {
      if (this.stepped) {
        const n = this.kind === 'choice' ? this.options.length : this.kind === 'bool' ? 2 : this.max - this.min + 1;
        const lo = this.kind === 'int' ? this.min : 0;
        this.base = lo + (((this.base - lo + steps) % n) + n) % n;
      } else this.base = this.fromNorm(this.norm + steps * 0.01);
      if (this.onChange) this.onChange(this.value);
    }
    reset() { this.base = this.def; if (this.onChange) this.onChange(this.value); }
    get text() {
      const v = this.value;
      if (this.kind === 'choice') return this.options[v] ?? '';
      if (this.kind === 'bool') return v ? 'on' : 'off';
      return this.fmt ? this.fmt(v) : (Math.abs(v) < 10 ? v.toFixed(2) : v.toFixed(1));
    }
  }
  const ctl = {
    f: (label, min, max, def, o = {}) => new Control(label, { kind: 'f', min, max, def, ...o }),
    choice: (label, options, def = 0, o = {}) => new Control(label, { kind: 'choice', options, def, ...o }),
    int: (label, min, max, def, o = {}) => new Control(label, { kind: 'int', min, max, def, ...o }),
    bool: (label, def = 0, o = {}) => new Control(label, { kind: 'bool', min: 0, max: 1, def, ...o }),
  };

  /* ---------------------------------------------------------------- kit */
  const STICK_SPAN = 0.5, HAND_SPAN = 0.6, STORE_HOLD = 0.6;
  const rank = (pad) => (3 - (pad >> 2)) * 4 + (pad & 3);     // pad index <-> musical rank (own inverse)

  class PlayKit {
    /* cfg: { controls, hero: [[a,b],[c,d],...], layers: [nativeLabel], browse, routes:{sx,sy,hl,hr}, throws:[{control,to,label}] } */
    constructor(cfg) {
      this.cfg = cfg; this.controls = cfg.controls; this.menuControls = cfg.menu || cfg.controls;
      this.layers = [{ kind: 'native', label: cfg.layers?.[0] ?? 'PLAY' }, { kind: 'controls', label: 'CONTROLS' }, { kind: 'moments', label: 'MOMENTS' }];
      if (cfg.throws && cfg.throws.length) this.layers.push({ kind: 'throws', label: 'THROWS' });
      if (cfg.layers && cfg.layers.length > 1) this.layers.splice(0, 1, ...cfg.layers.map((l) => ({ kind: 'native', label: l })));
      // effects and some generators open on their throws layer and have no native pad layer at all
      if (cfg.native === false && cfg.throws && cfg.throws.length) this.layers = [{ kind: 'throws', label: 'THROWS' }, { kind: 'controls', label: 'CONTROLS' }, { kind: 'moments', label: 'MOMENTS' }];
      this.layerIdx = 0; this.focus = 0; this.menu = false; this.menuSel = 0; this.status = ''; this.statusFrames = 0;
      this.routes = { ...(cfg.routes || {}) }; this.stick = [0, 0]; this.hands = [0, 0];
      this.moments = Array(16).fill(null); this.heldFor = Array(16).fill(0); this.prev = Array(16).fill(false);
      this.thrown = new Map(); this.grabbed = 0; this.peek = false;
      this.hero = cfg.hero && cfg.hero.length ? cfg.hero : [[0, 1]];
      this.flat = this.hero.flat();
    }
    get layer() { return this.layers[this.layerIdx]; }
    get native() { return this.layer.kind === 'native'; }
    flash(msg) { this.status = msg; this.statusFrames = 90; }
    nextLayer() { this.releaseThrows(); this.layerIdx = (this.layerIdx + 1) % this.layers.length; this.flash(this.layer.label); }
    focusedControl() { return this.controls[this.flat[this.focus % this.flat.length]]; }
    pair() { return this.hero[Math.floor(this.focus / 2) % this.hero.length]; }
    releaseThrows() { this.thrown.forEach((v, c) => { c.base = v; if (c.onChange) c.onChange(c.value); }); this.thrown.clear(); }

    /* One frame. `native(i, down)` is called for pad edges only while the native layer is showing.
       Returns whether the menu is up (the app then draws the list instead of the play view). */
    tick(inp, native) {
      if (this.statusFrames > 0 && --this.statusFrames === 0) this.status = '';
      if (inp.r1Edge) { this.menu = !this.menu; this.menuSel = Math.min(this.menuSel, this.menuControls.length - 1); }
      const p = inp.ptr;
      const hit = (regions) => p && (regions || []).find(r => p.x >= r.x && p.x < r.x+r.w && p.y >= r.y && p.y < r.y+r.h);
      if (p && p.type === 'down' && this.menu && hit([this.touchBack].filter(Boolean))) { this.menu=false; this.touchPad=null; this.touchDial=null; return false; }
      if (p && p.type === 'down' && !this.menu) {
        const action = hit(this.touchActions);
        if (action) { if (action.kind === 'layer') this.nextLayer(); else this.menu = true; }
        const pad = hit(this.touchPads); this.touchPad = pad ? pad.i : null;
        const expression = hit([this.touchStick,...(this.touchHands || [])].filter(Boolean));
        this.touchExpression = expression ? {...expression,px:p.x,py:p.y} : null;
      }
      if (p && (p.type === 'up' || p.type === 'cancel')) { this.touchPad = null; this.touchExpression = null; }
      if (p && this.touchExpression) {this.touchExpression.px=p.x;this.touchExpression.py=p.y;}
      inp = {...inp, held: inp.held.slice()};
      if (this.touchPad != null) inp.held[this.touchPad] = true;
      if (this.touchExpression) {
        const e = this.touchExpression;
        if (e.kind === 'stick') inp.stick = [clamp((e.px-e.x)/e.w*2-1,-1,1),clamp((e.py-e.y)/e.h*2-1,-1,1)];
        else {inp.hands=inp.hands.slice();inp.hands[e.i]=clamp(1-(e.py-e.y)/e.h,0,1);}
      }
      if (this.menu) {
        if (p && p.type === 'down') { const row = hit(this.touchRows); this.touchRow = row ? {i:row.i,y:p.y} : null; if(row)this.menuSel=row.i; }
        if (p && p.type === 'move' && this.touchRow) { const delta = Math.trunc((this.touchRow.y-p.y)/4); if(delta) {this.menuControls[this.touchRow.i].nudge(delta);this.touchRow.y=p.y;} }
        if (p && (p.type === 'up' || p.type === 'cancel')) this.touchRow=null;
        const n = this.menuControls.length;
        if (inp.navY) this.menuSel = clamp(this.menuSel + inp.navY, 0, n - 1);
        const c = this.menuControls[this.menuSel];
        if (inp.navX) c.nudge(inp.navX);
        if (inp.selectHold) c.reset();
        this.prev = inp.held.slice();
        return true;
      }
      const pointer = inp.ptr;
      if (pointer && pointer.type === 'down') {
        const hit = (this.touchDials || []).find(d => pointer.x >= d.x && pointer.x < d.x + d.w && pointer.y >= d.y && pointer.y < d.y + d.h);
        this.touchDial = hit ? { cid: hit.cid, y: pointer.y, norm: this.controls[hit.cid].norm } : null;
        if (hit && this.flat.includes(hit.cid)) this.focus = this.flat.indexOf(hit.cid);
      }
      if (this.touchDial && pointer) {
        if (pointer.type !== 'cancel') this.controls[this.touchDial.cid].norm = this.touchDial.norm + (this.touchDial.y - pointer.y) / 160;
        if (pointer.type === 'up' || pointer.type === 'cancel') this.touchDial = null;
      }
      this.peek = inp.l1;
      // dials: D-pad left/right turns the focused dial, SELECT moves the focus, up/down browses
      if (inp.select) { this.focus = (this.focus + 1) % this.flat.length; }
      const fc = this.focusedControl();
      if (inp.navX) fc.nudge(inp.navX);
      if (inp.selectHold) { fc.reset(); this.flash(`${fc.label} reset`); }
      if (inp.navY && this.cfg.browse != null) this.controls[this.cfg.browse].nudge(-inp.navY);

      // expression: the stick pushes its routed controls, hands push theirs, then everything returns home
      const push = (idx, amount) => { if (idx != null && this.controls[idx] && !this.controls[idx].stepped) this.controls[idx].off = clamp(this.controls[idx].off + amount, -1, 1); };
      this.controls.forEach((c) => { c.off = 0; });
      const r = this.routes;
      push(r.sx, inp.stick[0] * STICK_SPAN); push(r.sy, -inp.stick[1] * STICK_SPAN);
      push(r.hl, inp.hands[0] * HAND_SPAN); push(r.hr, inp.hands[1] * HAND_SPAN);
      this.controls.forEach((c) => { if (c.off && c.onChange) c.onChange(c.value); });
      this.stick = inp.stick.slice(); this.hands = inp.hands.slice();

      // pads
      const kind = this.layer.kind;
      for (let p = 0; p < 16; p++) {
        const rk = rank(p), down = inp.held[p] && !this.prev[p], up = !inp.held[p] && this.prev[p];
        if (kind === 'native') { if (down || up) native(p, down, inp.pressure ? inp.pressure[p] : 1); }
        else if (kind === 'controls') { if (down && rk < this.controls.length) { this.focus = Math.max(0, this.flat.indexOf(rk) >= 0 ? this.flat.indexOf(rk) : this.focus); this.grabbed = rk; this.flash(`${this.controls[rk].label}`); } }
        else if (kind === 'moments') {
          if (inp.held[p]) { this.heldFor[p] += inp.dt; if (this.heldFor[p] >= STORE_HOLD && this.heldFor[p] - inp.dt < STORE_HOLD) { this.moments[rk] = this.controls.map((c) => c.base); this.flash(`Moment ${rk + 1} stored`); } }
          if (up) {
            if (this.heldFor[p] < STORE_HOLD && this.moments[rk]) { this.moments[rk].forEach((v, i) => { this.controls[i].base = v; if (this.controls[i].onChange) this.controls[i].onChange(this.controls[i].value); }); this.flash(`Moment ${rk + 1}`); }
            this.heldFor[p] = 0;
          }
        } else if (kind === 'throws') {
          const t = this.cfg.throws[rk];
          if (t && t.hold) { if (down) t.hold(true); if (up) t.hold(false); }
          else if (t && down) { const c = this.controls[t.control]; this.thrown.set(c, c.base); c.base = c.fromNorm(t.to); if (c.onChange) c.onChange(c.value); }
          else if (t && up) { const c = this.controls[t.control]; if (this.thrown.has(c)) { c.base = this.thrown.get(c); this.thrown.delete(c); if (c.onChange) c.onChange(c.value); } }
        }
      }
      this.prev = inp.held.slice();
      return false;
    }

    /* pad LED / map state: 0 off, 1 blue, 2 yellow, 3 green, 4 red */
    padState(p, nativeState) {
      const rk = rank(p), held = this.prev[p];
      switch (this.layer.kind) {
        case 'native': return nativeState ? nativeState(p) : 0;
        case 'controls': return rk === this.grabbed ? 3 : rk < this.controls.length ? 1 : 0;
        case 'moments': return held ? 4 : this.moments[rk] ? 3 : 0;
        default: { const t = this.cfg.throws[rk]; return !t ? 0 : held ? 4 : 2; }
      }
    }
    padLabel(p, nativeLabel) {
      const rk = rank(p);
      switch (this.layer.kind) {
        case 'native': return nativeLabel ? nativeLabel(p) : '';
        case 'controls': return rk < this.controls.length ? this.controls[rk].label : '';
        case 'moments': return this.moments[rk] ? `${rk + 1} ●` : `${rk + 1} ·`;
        default: { const t = this.cfg.throws[rk]; return t ? t.label : ''; }
      }
    }
    routeLabel(i) { return i != null && this.controls[i] ? this.controls[i].label : '-'; }

    /* ---- drawing ---- */
    column(fb, x, y, w, h, pal, o = {}) {
      const ink = pal.ink, dim = pal.dim, bg = pal.bg;
      fb.rect(x, y, w, h, bg);
      const chip = `PADS: ${this.layer.label}`;
      fb.rect(x, y, chip.length * 6 + 8, 14, pal.accent);
      fb.text(chip, x + 4, y + 10, 's', bg);
      fb.text('R1 menu', x + w - 42, y + 10, 's', dim);

      const pages = this.hero.length, first = pages ? (Math.floor(this.focus / 2) % pages) & ~1 : 0;
      const ids = [];
      for (let p = first; p < Math.min(first + 2, pages); p++) ids.push(...this.hero[p]);
      const fc = this.flat[this.focus % this.flat.length], pr = this.pair();
      const dw = ((w - 8) / 2) | 0;
      ids.slice(0, 4).forEach((cid, i) => {
        const c = this.controls[cid], dx = x + (i % 2) * (dw + 8), dy = y + 20 + ((i / 2) | 0) * 30;
        const marker = fc === cid ? 2 : pr.includes(cid) ? 1 : 0;
        let lx = dx;
        if (marker) { fb.disc(dx, dy, 11, pal.accent); fb.text(String(marker), dx + 3, dy + 9, 's', bg); lx += 14; }
        fb.text(c.label.slice(0, Math.max(0, ((dw - (lx - dx)) / 6) | 0)), lx, dy + 9, 's', ink);
        const v = c.text.slice(0, (dw / 6) | 0);
        fb.rect(dx, dy + 13, dw, 4, pal.faint);
        fb.rect(dx, dy + 13, (dw * c.effNorm) | 0, 4, marker ? pal.accent : dim);
        fb.text(v, dx + dw - v.length * 6, dy + 26, 's', dim);
      });
      let py = y + 82;
      if (!ids.includes(fc)) { fb.text(`2: ${this.controls[fc].label}  ${this.controls[fc].text}`.slice(0, (w / 6) | 0), x, py + 8, 's', ink); py += 12; }

      // pad map
      const cw = ((w - 6) / 4) | 0, ch = 24;
      for (let i = 0; i < 16; i++) {
        const cx = x + (i % 4) * (cw + 2), cy = py + ((i / 4) | 0) * (ch + 2);
        const st = o.padState ? o.padState(i) : 0;
        let fill = bg, tc = ink;
        if (st === 3 || st === 4) { fill = pal.accent; tc = bg; } else if (st === 1 || st === 2) fill = pal.faint;
        fb.rect(cx, cy, cw, ch, fill); fb.frame(cx, cy, cw, ch, dim);
        if (st === 2) fb.rect(cx + 1, cy + ch - 4, cw - 2, 3, dim);
        const label = o.padLabel ? o.padLabel(i) : '';
        fb.text(label.slice(0, ((cw - 4) / 6) | 0), cx + 3, cy + 15, 's', tc);
      }
      py += 4 * (ch + 2) + 4;

      // stick and hands
      const s = 40;
      fb.frame(x, py, s, s, dim); fb.vline(x + s / 2, py, s, pal.faint); fb.hline(x, py + s / 2, s, pal.faint);
      fb.disc(x + s / 2 + Math.trunc(clamp(this.stick[0], -1, 1) * 16) - 3, py + s / 2 - Math.trunc(clamp(this.stick[1], -1, 1) * 16) - 3, 7, pal.accent);
      const sl = `${this.routeLabel(this.routes.sx)} / ${this.routeLabel(this.routes.sy)}`;
      fb.text(sl.slice(0, ((w - s - 60) / 6) | 0), x + s + 6, py + 10, 's', dim);
      [[0, x + w - 40], [1, x + w - 18]].forEach(([hi, hx]) => {
        const v = clamp(this.hands[hi], 0, 1);
        fb.rect(hx, py, 12, s - 12, pal.faint);
        const hh = ((s - 12) * v) | 0;
        fb.rect(hx, py + s - 12 - hh, 12, hh, pal.accent);
        fb.text(hi ? 'R' : 'L', hx + 3, py + s, 's', dim);
      });
      const line = this.status || (o.line ? o.line() : '');
      fb.text(line.slice(0, ((w - s - 50) / 8) | 0), x + s + 6, py + 30, 'm', pal.accent);
    }

    /* the R1 menu: "> name: value" rows, a chip behind the selected one */
    drawMenu(fb, title, pal, extraRows = [], x = 16, y = 44, rowH = 24, visible = 11) {
      fb.text(title, 16, 30, 'm', pal.accent);
      if (this.status) fb.text(this.status, 130, 30, 's', pal.dim);
      const rows = this.menuControls.map((c) => [c.label, c.text]).concat(extraRows);
      const n = rows.length, sel = this.menuSel;
      const start = clamp(sel - ((visible / 2) | 0), 0, Math.max(0, n - visible)), end = Math.min(n, start + visible);
      for (let i = start; i < end; i++) {
        const yy = y + (i - start) * rowH, text = `${i === sel ? '>' : ' '} ${rows[i][0]}: ${rows[i][1]}`;
        if (i === sel) { fb.roundRect(x - 3, yy - 13, text.length * 8 + 6, 19, 3, pal.chip || pal.faint); }
        fb.text(text, x, yy, 'm', i === sel ? pal.ink : pal.dim);
      }
      if (start > 0) fb.text('^ more', x, y - rowH, 'm', pal.dim);
      if (end < n) fb.text('v more', x, y + visible * rowH, 'm', pal.dim);
    }
  }

  R.ctl = ctl; R.PlayKit = PlayKit; R.rank = rank;
})();
