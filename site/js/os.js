/* The device shell: boot splash, home launcher, mixer, help, and the host that runs an app.
   Mirrors src/os.rs: F1 home, F2 the app's pad layer, F3 its transport, F4 mixer, a hint line and
   an F-key bar under every app screen. */
(() => {
  'use strict';
  const R = window.ROVE;
  const A = R.audio;
  const { c565, blend } = R;
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));

  const registry = [];
  R.registerApp = (def) => registry.push(def);

  const BAR_H = 17, HINT_H = 14;
  const HINT_COL = c565(18, 36, 18), DIM_COL = c565(8, 12, 8), ACCENT = c565(11, 59, 15);
  const CATS = ['ALL APPS', 'INSTRUMENTS', 'SEQUENCING', 'EFFECTS'];
  const CAT_OF = { instrument: 'INSTRUMENTS', sequencer: 'SEQUENCING', effect: 'EFFECTS' };

  const HOME = { bg: '#121B1B', ink: '#F1F0E6', dim: '#8FA39B', chip: '#18231F', sel: '#B7D6C5', selInk: '#121B1B' };
  const sans = '"Space Grotesk", "Helvetica Neue", Arial, sans-serif', mono = '"JetBrains Mono", ui-monospace, Menlo, monospace';

  class OS {
    constructor(canvas) {
      this.fb = new R.FB(canvas);
      this.mode = 'boot'; this.t = 0; this.bootAt = performance.now();
      this.apps = registry.slice().sort((a, b) => a.name.localeCompare(b.name)).map((d) => ({ def: d, inst: null }));
      this.cat = 0; this.sel = 0; this.active = null; this.prevMode = 'home';
      this.mixSel = 0; this.helpPage = 0; this.say = () => {}; this.lastSpoken = '';
      this.nav = { dir: null, t: 0, ticks: 0 };           // D-pad hold-to-repeat
      this.raw = {
        held: Array(16).fill(false), f: [false, false, false, false], l1: false, r1: false,
        stick: [0, 0], hands: [0, 0], dir: { up: false, down: false, left: false, right: false }, select: false, ptr: null, pressure: Array(16).fill(1),
      };
      this.pointerEvents = [];
      this.edges = { f: [0, 0, 0, 0], l1: 0, r1: 0, select: 0, stickClick: 0, ptr: null };
      this.selectHeld = 0; this.selectFired = false; this.prevHeld = Array(16).fill(false);
      this.logos = {};
      ['szyk', 'rove'].forEach((n) => { const im = new Image(); im.src = `assets/${n}-logo-light.svg`; this.logos[n] = im; });
      this.visible = () => this.apps.filter((a) => this.cat === 0 || CAT_OF[a.def.category] === CATS[this.cat]);
    }

    /* ---------------- app lifecycle ---------------- */
    open(a) {
      if (!A.ctx) A.init();
      if (!a.inst) a.inst = a.def.create(this);
      if (this.active && this.active !== a) this.active.inst.leave && this.active.inst.leave();
      this.active = a; this.mode = 'app';
      a.inst.enter && a.inst.enter();
      this.say(`${a.def.name} opened. ${a.def.desc}`);
    }
    home() { if (this.active && this.active.inst.leave) this.active.inst.leave(); this.active = null; this.mode = 'home'; this.say('Home'); }
    runningApps() { return this.apps.filter((a) => a.inst && a.inst.running && a.inst.running() === true); }

    /* ---------------- input -> one frame's worth ---------------- */
    buildInput(dt) {
      const r = this.raw, e = this.edges;
      const down = r.held.map((h, i) => h && !this.prevHeld[i]), up = r.held.map((h, i) => !h && this.prevHeld[i]);
      this.prevHeld = r.held.slice();
      // D-pad: one step on press, repeat after 250 ms then every 100 ms; 1 -> 5 -> 20 steps the longer it is held
      let navX = 0, navY = 0;
      const cur = r.dir.up ? 'up' : r.dir.down ? 'down' : r.dir.left ? 'left' : r.dir.right ? 'right' : null;
      const stepOf = (d) => ({ up: [0, -1], down: [0, 1], left: [-1, 0], right: [1, 0] }[d]);
      if (cur !== this.nav.dir) { this.nav = { dir: cur, t: 0, ticks: 0 }; if (cur) { const [x, y] = stepOf(cur); navX += x; navY += y; } }
      else if (cur) {
        this.nav.t += dt;
        const iv = this.nav.ticks === 0 ? 0.25 : 0.1;
        if (this.nav.t >= iv) { this.nav.t -= iv; this.nav.ticks++; const m = this.nav.ticks < 6 ? 1 : this.nav.ticks < 16 ? 5 : 20; const [x, y] = stepOf(cur); navX += x * m; navY += y * m; }
      }
      // SELECT: tap selects, holding half a second resets
      let select = false, selectHold = false;
      if (r.select) { this.selectHeld += dt; if (this.selectHeld >= 0.5 && !this.selectFired) { this.selectFired = true; selectHold = true; } }
      else { if (this.selectHeld > 0 && !this.selectFired && this.selectHeld > 0) select = true; this.selectHeld = 0; this.selectFired = false; }
      const inp = {
        held: r.held.slice(), down, up, pressure: r.pressure, navX, navY, select, selectHold,
        f: e.f.map((v) => v > 0), l1: r.l1, r1: r.r1, l1Edge: e.l1 > 0, r1Edge: e.r1 > 0,
        stick: r.stick.slice(), stickClick: e.stickClick > 0, hands: r.hands.slice(), ptr: this.pointerEvents.length ? this.pointerEvents.shift() : e.ptr, ptrNow: r.ptr, dt,
      };
      e.f = [0, 0, 0, 0]; e.l1 = e.r1 = e.stickClick = 0; e.ptr = null;
      return inp;
    }

    tick(dt) {
      this.t += dt;
      const inp = this.buildInput(dt);
      if (this.mode === 'boot') {
        if (performance.now() - this.bootAt > 2600 || inp.f.some(Boolean) || inp.select || inp.down.some(Boolean)) { this.mode = 'home'; }
        return;
      }
      // F4 mixer from anywhere, F1 home from any app
      if (inp.f[3]) { if (this.mode === 'mixer') this.mode = this.prevMode; else { this.prevMode = this.mode === 'help' ? 'home' : this.mode; this.mode = 'mixer'; this.say('Mixer'); } return this.tickMixer(inp); }
      if (this.mode === 'mixer') return this.tickMixer(inp);
      if (this.mode === 'help') { if (inp.f[0] || inp.select || inp.r1Edge) this.mode = 'home'; if (inp.navY) this.helpPage = clamp(this.helpPage + Math.sign(inp.navY), 0, 1); return; }
      if (this.mode === 'home') return this.tickHome(inp);
      const a = this.active;
      if (inp.f[0]) return this.home();
      if (inp.f[1] && a.inst.f2) a.inst.f2();
      if (inp.f[2] && a.inst.toggleRunning && a.inst.running && a.inst.running() !== null) a.inst.toggleRunning();
      a.inst.tick(inp);
      // describe the focused thing for screen readers, a little after it changes
      if (a.inst.speak) { const s = a.inst.speak(); if (s !== this.lastSpoken) { this.lastSpoken = s; clearTimeout(this._sp); this._sp = setTimeout(() => this.say(s), 500); } }
    }

    tickHome(inp) {
      const list = this.visible();
      if (inp.f[0]) { this.mode = 'help'; this.helpPage = 0; this.say('Help'); return; }
      if (inp.f[1]) { this.cat = (this.cat + 1) % CATS.length; this.sel = 0; }
      if (inp.navX) { this.cat = ((this.cat + Math.sign(inp.navX)) + CATS.length) % CATS.length; this.sel = 0; }
      if (inp.navY) this.sel = clamp(this.sel + Math.sign(inp.navY), 0, Math.max(0, this.visible().length - 1));
      if (inp.select || inp.r1Edge) { const a = this.visible()[this.sel]; if (a) this.open(a); }
      const p = inp.ptr;
      if (p && p.type === 'down') this.homeTouch = { x: p.x, y: p.y, selected: this.sel, dragged: false };
      if (p && p.type === 'cancel') this.homeTouch = null;
      const touch = this.homeTouch;
      if (touch && p) {
        if (Math.abs(p.y - touch.y) > 8 || Math.abs(p.x - touch.x) > 8) touch.dragged = true;
        if (touch.x >= 370 && touch.x <= 388) {
          this.sel = Math.round(clamp((p.y - 72) / 244, 0, 1) * Math.max(0, list.length - 1));
        } else if (touch.x >= 142 && touch.x < 370 && touch.dragged) {
          this.sel = clamp(clamp(touch.selected - 3, 0, Math.max(0, list.length - 8)) + 3 + Math.round((touch.y - p.y) / 31), 0, Math.max(0, list.length - 1));
        }
        if (p.type === 'up') {
          if (!touch.dragged && touch.x >= 142 && touch.x < 370 && touch.y >= 72 && touch.y < 320) {
            const start = clamp(touch.selected - 3, 0, Math.max(0, list.length - 8));
            const index = start + Math.floor((touch.y - 72) / 31);
            if (list[index]) { this.sel = index; this.open(list[index]); }
          } else if (!touch.dragged && touch.x >= 388 && touch.y >= 288 && touch.y <= 316 && list[this.sel]) {
            this.open(list[this.sel]);
          } else if (!touch.dragged && touch.x >= 18 && touch.x < 124 && touch.y >= 72 && touch.y < 72 + CATS.length * 25) {
            this.cat = Math.floor((touch.y - 72) / 25); this.sel = 0;
          }
          this.homeTouch = null;
        }
      }
      // pads 1-16 jump straight to the first 16 apps in the list, a convenience the simulator also has
      inp.down.forEach((d, i) => { if (d && list[i]) { this.sel = i; this.open(list[i]); } });
    }
    tickMixer(inp) {
      const ch = [...A.channels.values()].filter((c) => c.name !== 'Demo');
      const rows = 1 + ch.length;
      if (inp.navY) this.mixSel = clamp(this.mixSel + Math.sign(inp.navY), 0, rows - 1);
      if (inp.navX) {
        if (this.mixSel === 0) A.setVolume(clamp(A.volume + inp.navX * 0.01, 0, 1));
        else ch[this.mixSel - 1].setLevel(ch[this.mixSel - 1].level + inp.navX * 0.01);
      }
      if (inp.select && this.mixSel > 0) { const c = ch[this.mixSel - 1]; c.muted = !c.muted; c.setLevel(c.level); }
      if (inp.f[0]) { this.mode = this.prevMode === 'app' && this.active ? 'app' : 'home'; if (this.mode === 'home') this.active = null; }
    }

    /* ---------------- drawing ---------------- */
    draw() {
      const fb = this.fb;
      if (this.mode === 'boot') return this.drawBoot(fb);
      if (this.mode === 'home') return this.drawHome(fb);
      if (this.mode === 'help') return this.drawHelp(fb);
      if (this.mode === 'mixer') { this.drawMixer(fb); return this.drawBars(fb, ['HOME', '', '', 'BACK'], { hint: '↑↓ CHANNEL  ·  ◀▶ LEVEL  ·  SELECT MUTE' }); }
      const a = this.active;
      a.inst.draw(fb);
      const inst = a.inst, f2 = inst.gridModeLabel ? inst.gridModeLabel() : null;
      const run = inst.running ? inst.running() : null, tr = inst.transportAction ? inst.transportAction() : '';
      if (inst.view === 'full') { /* the app owns the whole screen, bars included */ }
      else if (inst.view === 'slint') { R.slint.hint(fb, inst.pal, inst.hint()); R.slint.bar(fb, inst.pal, ['HOME', f2 ? `PADS: ${f2}` : '', tr, 'MIXER'], run === true); }
      else this.drawBars(fb, ['Home', f2 ? `PADS: ${f2}` : '', tr, 'Mixer'], { hint: inst.hint ? inst.hint() : '', f3Lit: run === true });
    }

    drawBars(fb, labels, { hint = '', f3Lit = false, plain = false } = {}) {
      const barY = 360 - BAR_H;
      fb.rect(0, barY - HINT_H, 640, HINT_H, '#000000');
      fb.text(hint, 6, barY - 3, 's', HINT_COL);
      fb.hline(0, barY, 640, DIM_COL);
      fb.rect(0, barY + 1, 640, BAR_H - 1, '#000000');
      const seg = 640 / 4;
      labels.forEach((label, i) => {
        const x = Math.trunc(i * seg + 6), y = 356;
        if (!label) { fb.text(`F${i + 1}`, x, y, 's', c565(8, 12, 8)); return; }
        const text = `F${i + 1}: ${label}`, lit = i === 2 && f3Lit;
        if (lit) fb.rect(x - 3, y - 12, text.length * 6 + 6, 16, ACCENT);
        fb.text(text, x, y, 's', lit ? '#000000' : HINT_COL);
      });
    }

    drawBoot(fb) {
      const t = performance.now() - this.bootAt;
      fb.clear('#000000');
      const which = t < 1200 ? 'szyk' : 'rove', local = which === 'szyk' ? t : t - 1200;
      const alpha = clamp(Math.min(local / 300, (which === 'szyk' ? 1200 - t : 1400 - local) / 300), 0, 1);
      const im = this.logos[which];
      if (im.complete && im.naturalWidth) {
        const w = which === 'szyk' ? 300 : 280, h = w * (90 / 384);
        fb.g.globalAlpha = alpha; fb.image(im, (640 - w) / 2, (360 - h) / 2, w, h); fb.g.globalAlpha = 1;
      }
    }

    ttf(fb, str, x, y, size, color, { weight = 400, family = sans, align = 'left', spacing = 0 } = {}) {
      const g = fb.g; g.font = `${weight} ${size}px ${family}`; g.fillStyle = color; g.textAlign = align; g.textBaseline = 'alphabetic';
      if ('letterSpacing' in g) g.letterSpacing = `${spacing}px`;
      g.fillText(str, x, y);
      if ('letterSpacing' in g) g.letterSpacing = '0px';
    }

    drawHome(fb) {
      const H = HOME, list = this.visible();
      fb.clear(H.bg);
      this.ttf(fb, 'ROVE', 18, 38, 27, H.ink, { weight: 700, spacing: 0.4 });
      this.ttf(fb, 'SELECT AN APP', 622, 34, 12, H.dim, { family: mono, align: 'right', spacing: 0.6 });
      this.ttf(fb, 'WORKSPACE', 18, 62, 11, H.dim, { family: mono, spacing: 0.6 });
      CATS.forEach((c, i) => {
        const y = 72 + i * 25, on = i === this.cat;
        fb.roundRect(18, y, 106, 21, 3, on ? H.sel : H.chip);
        this.ttf(fb, c, 26, y + 15, 12, on ? H.selInk : H.ink, { weight: on ? 600 : 500, spacing: 0.3 });
      });
      this.ttf(fb, `${this.apps.length} INSTALLED`, 18, 301, 11, H.dim, { family: mono, spacing: 0.4 });

      this.ttf(fb, `${list.length} APPS`, 142, 62, 11, H.dim, { family: mono, spacing: 0.6 });
      const visible = 8, start = clamp(this.sel - 3, 0, Math.max(0, list.length - visible));
      list.slice(start, start + visible).forEach((a, i) => {
        const y = 72 + i * 31, on = start + i === this.sel;
        fb.roundRect(142, y, 228, 27, 4, on ? H.sel : H.bg);
        this.ttf(fb, a.def.name, 154, y + 19, 15, on ? H.selInk : H.ink, { weight: on ? 600 : 400 });
        if (a.inst && a.inst.running && a.inst.running() === true) fb.disc(352, y + 9, 8, on ? H.selInk : '#C9E8CF');
      });
      fb.roundRect(374, 72, 8, 244, 4, '#344640');
      const thumb = Math.min(244, Math.max(28, 244 * visible / Math.max(1, list.length)));
      fb.roundRect(374, 72 + (244 - thumb) * start / Math.max(1, list.length - visible), 8, thumb, 4, H.sel);
      if (start > 0) this.ttf(fb, '↑', 356, 66, 12, H.dim, { family: mono });
      if (start + visible < list.length) this.ttf(fb, '↓ MORE APPS', 370, 322, 10, H.dim, { family: mono, align: 'right' });

      const cur = list[this.sel];
      if (cur) {
        this.ttf(fb, cur.def.category === 'effect' ? 'Effects' : cur.def.category === 'sequencer' ? 'Sequencing' : 'Instruments', 388, 62, 11, H.dim, { family: mono });
        let hsh = 0; for (const ch of cur.def.id) hsh = (hsh * 31 + ch.charCodeAt(0)) >>> 0;
        for (let i = 0; i < 8; i++) { hsh = (hsh * 1664525 + 1013904223) >>> 0; const bh = 12 + (hsh % 33); fb.roundRect(388 + i * 20, 135 - bh, 15, bh, 2, i % 3 === 2 ? '#6B8574' : '#3A4A40'); }
        ['S', 'Z', 'Y', 'K'].forEach((ch, i) => this.ttf(fb, ch, 594, 100 + i * 12, 11, H.dim, { family: mono, weight: 500 }));
        this.ttf(fb, cur.def.name, 388, 175, 28, '#F2F7F3', { weight: 700 });
        this.wrap(fb, cur.def.desc, 388, 232, 232, 11, 14, H.ink);
        fb.roundRect(388, 288, 234, 28, 4, '#1B221D');
        this.ttf(fb, 'R1  OPEN APPLICATION', 505, 306, 11, H.ink, { family: mono, align: 'center', spacing: 0.4 });
      }
      fb.hline(0, 324, 640, '#1F2823');
      ['F1  HELP', 'F2  CATEGORY', 'F3', 'F4  MIXER'].forEach((l, i) => this.ttf(fb, l, 2 + i * 160, 346, 14, '#6F8D78', { family: mono, spacing: 0.3 }));
    }

    wrap(fb, text, x, y, w, size, lh, color) {
      const g = fb.g; g.font = `400 ${size}px ${sans}`;
      const words = text.split(' '); let line = '', yy = y;
      words.forEach((wd) => {
        const t = line ? `${line} ${wd}` : wd;
        if (g.measureText(t).width > w && line) { this.ttf(fb, line, x, yy, size, color); line = wd; yy += lh; } else line = t;
      });
      if (line) this.ttf(fb, line, x, yy, size, color);
    }

    drawHelp(fb) {
      const H = HOME; fb.clear(H.bg);
      this.ttf(fb, 'HELP', 18, 38, 27, H.ink, { weight: 700 });
      this.ttf(fb, this.helpPage === 0 ? '1 / 2' : '2 / 2', 622, 34, 12, H.dim, { family: mono, align: 'right' });
      const rows = this.helpPage === 0 ? [
        ['D-PAD  ← →', 'Turn the focused dial (hold: faster)'],
        ['D-PAD  ↑ ↓', 'Browse presets / move in lists'],
        ['SELECT', 'Next dial. Hold half a second: reset it'],
        ['PADS 1-16', 'Play. Row-major, bottom-left is the root'],
        ['STICK', 'Pushes the dials named under the pad map'],
        ['HAND L / R', 'Drag the sensors up: closer = more'],
      ] : [
        ['F1', 'Home (from an app). Help (from Home)'],
        ['F2', 'Cycles the pad layer: play, controls, moments, throws'],
        ['F3', 'Start / stop the app’s generator or player'],
        ['F4', 'Mixer: every running app’s channel'],
        ['L1 / R1', 'R1 opens the app’s full menu. L1 peeks at the dials'],
        ['SCREEN', 'Bebot and Vistas also answer the mouse'],
      ];
      rows.forEach(([a, b], i) => { this.ttf(fb, a, 18, 78 + i * 38, 12, H.ink, { family: mono, weight: 700 }); this.ttf(fb, b, 150, 78 + i * 38, 13, H.ink); });
      fb.hline(0, 324, 640, '#1F2823');
      this.ttf(fb, 'F1  BACK', 2, 346, 14, '#6F8D78', { family: mono }); this.ttf(fb, '↑↓  PAGE', 162, 346, 14, '#6F8D78', { family: mono });
    }

    drawMixer(fb) {
      const bg = c565(1, 5, 4), ink = c565(24, 56, 28), dim = c565(10, 26, 14);
      fb.clear(bg);
      fb.text('Mixer', 16, 30, 'xl', ink);
      const ch = [...A.channels.values()].filter((c) => c.name !== 'Demo');
      const rows = [`Master: ${Math.round(A.volume * 100)}%`, ...ch.map((c) => `${c.name}: ${c.muted ? 'muted' : Math.round(c.level * 100) + '%'}`)];
      rows.slice(0, 11).forEach((t, i) => {
        const y = 66 + i * 22, on = i === this.mixSel, s = `${on ? '>' : ' '} ${t}`;
        if (on) fb.roundRect(13, y - 13, s.length * 8 + 6, 19, 3, c565(4, 14, 18));
        fb.text(s, 16, y, 'm', on ? ink : dim);
      });
      if (!ch.length) fb.text('  (no apps have made sound yet)', 16, 88, 'm', dim);
      // vertical strips
      const strips = [{ n: 'MST', v: A.volume, lvl: A.analyser ? this.masterLevel() : 0 }, ...ch.slice(0, 8).map((c) => ({ n: c.name.slice(0, 3).toUpperCase(), v: Math.min(1, c.level), lvl: c.rms() * 3 }))];
      strips.forEach((s, i) => {
        const x = 640 - 20 - (strips.length - i) * 34, top = 60, h = 220;
        fb.frame(x, top, 24, h, dim);
        fb.rect(x + 1, top + h - 1 - Math.round((h - 2) * clamp(s.lvl, 0, 1)), 22, Math.round((h - 2) * clamp(s.lvl, 0, 1)), c565(6, 30, 20));
        fb.hline(x - 2, top + h - Math.round(h * s.v), 28, ink);
        fb.text(s.n, x + 1, top + h + 14, 's', dim);
      });
    }
    masterLevel() {
      if (!A.analyser) return 0;
      if (!this._mb) this._mb = new Float32Array(512);
      A.analyser.getFloatTimeDomainData(this._mb); let s = 0; for (let i = 0; i < 512; i++) s += this._mb[i] ** 2; return Math.sqrt(s / 512) * 3;
    }

    /* the physical pad LEDs mirror the app's pad state: 0 off, 1 blue, 2 yellow, 3 green, 4 red */
    leds() {
      if (this.mode === 'home') return Array.from({ length: 16 }, (_, i) => (this.visible()[i] ? 3 : 0));
      if (this.mode === 'app' && this.active && this.active.inst.ledStates) return this.active.inst.ledStates();
      return Array(16).fill(0);
    }
  }

  R.OS = OS;
})();
