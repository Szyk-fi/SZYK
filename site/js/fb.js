/* Framebuffer for the 640x360 screen. Mirrors the calls the firmware's drawing code makes
   (embedded-graphics): 1 px strokes drawn inside the rectangle, Spleen bitmap text placed by
   baseline, Rgb565 colours. Apps are ported against this so their screens line up pixel for pixel. */
(() => {
  'use strict';
  const W = 640, H = 360;
  const SANS = '"Space Grotesk", "Helvetica Neue", Arial, sans-serif', MONO = '"JetBrains Mono", ui-monospace, Menlo, monospace';

  // Rgb565::new(r5, g6, b5) -> CSS colour, scaled the way the real display driver does
  const c565 = (r, g, b) => {
    const to8 = (v, max) => Math.round((v * 255) / max);
    return '#' + [to8(r, 31), to8(g, 63), to8(b, 31)].map((v) => v.toString(16).padStart(2, '0')).join('');
  };
  const hexToRgb = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
  const rgbToHex = (a) => '#' + a.map((v) => Math.max(0, Math.min(255, Math.round(v))).toString(16).padStart(2, '0')).join('');
  const rgba = (hex, a) => { const [r, g, b] = hexToRgb(hex); return `rgba(${r},${g},${b},${a})`; };
  const blend = (a, b, t) => { const x = hexToRgb(a), y = hexToRgb(b); return rgbToHex(x.map((v, i) => v + (y[i] - v) * t)); };

  const FONTS = {
    s: { key: '6x12', baseline: 8 },     // dim labels, hints, captions
    m: { key: '8x16', baseline: 11 },    // the shared menu text
    l: { key: '12x24', baseline: 18 },
    xl: { key: '16x32', baseline: 25 },  // each app's title
  };
  const glyphCache = new Map();
  function glyphSheet(fontName, color) {
    const k = fontName + color;
    let sheet = glyphCache.get(k);
    if (sheet) return sheet;
    const f = window.SPLEEN[FONTS[fontName].key];
    sheet = document.createElement('canvas');
    sheet.width = 95 * f.w; sheet.height = f.h;
    const g = sheet.getContext('2d');
    g.fillStyle = color;
    const bytes = Math.ceil(f.w / 8);
    f.g.forEach((rows, gi) => {
      rows.forEach((v, y) => {
        for (let x = 0; x < f.w; x++) if ((v >> (bytes * 8 - 1 - x)) & 1) g.fillRect(gi * f.w + x, y, 1, 1);
      });
    });
    glyphCache.set(k, sheet);
    return sheet;
  }

  class FB {
    constructor(canvas) {
      this.canvas = canvas;
      this.g = canvas.getContext('2d', { alpha: false });
      this.g.imageSmoothingEnabled = false;
      // Fit the native drawing above the touchscreen action bar.
      this.g.fillStyle = '#000'; this.g.fillRect(0, 0, 800, 432);
      this.g.setTransform(1.2, 0, 0, 1.2, 16, 0);
    }
    clear(c) { this.g.fillStyle = c; this.g.fillRect(0, 0, W, H); }
    rect(x, y, w, h, c) { if (w > 0 && h > 0) { this.g.fillStyle = c; this.g.fillRect(x | 0, y | 0, w | 0, h | 0); } }
    frame(x, y, w, h, c, t = 1) {
      x |= 0; y |= 0; w |= 0; h |= 0;
      this.rect(x, y, w, t, c); this.rect(x, y + h - t, w, t, c);
      this.rect(x, y, t, h, c); this.rect(x + w - t, y, t, h, c);
    }
    hline(x, y, w, c) { this.rect(x, y, w, 1, c); }
    vline(x, y, h, c) { this.rect(x, y, 1, h, c); }
    line(x0, y0, x1, y1, c) {
      x0 |= 0; y0 |= 0; x1 |= 0; y1 |= 0;
      this.g.fillStyle = c;
      const dx = Math.abs(x1 - x0), dy = -Math.abs(y1 - y0), sx = x0 < x1 ? 1 : -1, sy = y0 < y1 ? 1 : -1;
      let err = dx + dy;
      for (let n = 0; n < 2000; n++) {
        this.g.fillRect(x0, y0, 1, 1);
        if (x0 === x1 && y0 === y1) break;
        const e2 = 2 * err;
        if (e2 >= dy) { err += dy; x0 += sx; }
        if (e2 <= dx) { err += dx; y0 += sy; }
      }
    }
    // filled circle with top-left (x, y) and diameter d
    disc(x, y, d, c) {
      this.g.fillStyle = c;
      const r = d / 2;
      for (let row = 0; row < d; row++) {
        const dy = row + 0.5 - r, half = Math.sqrt(Math.max(0, r * r - dy * dy));
        const x0 = Math.round(r - half), x1 = Math.round(r + half);
        this.g.fillRect(x + x0, y + row, x1 - x0, 1);
      }
    }
    ring(x, y, d, c, t = 1) {
      this.g.fillStyle = c;
      const r = d / 2, ri = Math.max(0, r - t);
      for (let row = 0; row < d; row++) {
        const dy = row + 0.5 - r;
        const ho = Math.sqrt(Math.max(0, r * r - dy * dy));
        const hi = Math.abs(dy) < ri ? Math.sqrt(Math.max(0, ri * ri - dy * dy)) : 0;
        const xo0 = Math.round(r - ho), xo1 = Math.round(r + ho);
        if (hi <= 0) this.g.fillRect(x + xo0, y + row, xo1 - xo0, 1);
        else {
          const xi0 = Math.round(r - hi), xi1 = Math.round(r + hi);
          this.g.fillRect(x + xo0, y + row, xi0 - xo0, 1);
          this.g.fillRect(x + xi1, y + row, xo1 - xi1, 1);
        }
      }
    }
    roundRect(x, y, w, h, rad, c) {
      this.g.fillStyle = c;
      rad = Math.min(rad, w / 2, h / 2);
      for (let row = 0; row < h; row++) {
        let inset = 0;
        if (row < rad) inset = rad - Math.sqrt(Math.max(0, rad * rad - (rad - row - 0.5) ** 2));
        else if (row >= h - rad) inset = rad - Math.sqrt(Math.max(0, rad * rad - (row - (h - rad) + 0.5) ** 2));
        const i = Math.round(inset);
        this.g.fillRect(x + i, y + row, w - 2 * i, 1);
      }
    }
    // text placed by baseline, like embedded-graphics Text::new
    text(s, x, y, font, c) {
      const f = window.SPLEEN[FONTS[font].key], sheet = glyphSheet(font, c), top = y - FONTS[font].baseline;
      let cx = x | 0;
      for (let i = 0; i < s.length; i++) {
        let code = s.charCodeAt(i);
        if (code < 32 || code > 126) code = 32;       // the firmware's mapping: anything else is blank
        if (code !== 32) this.g.drawImage(sheet, (code - 32) * f.w, 0, f.w, f.h, cx, top | 0, f.w, f.h);
        cx += f.w;
      }
    }
    // firmware-style text (kids_kit::text): y is the TOP edge, align -1 left / 0 centre / 1 right; 'huge' is the 16x32 face at 2x
    ktext(s, x, y, size, c, align = -1) {
      const font = { small: 's', medium: 'm', large: 'xl', huge: 'xl' }[size], f = window.SPLEEN[FONTS[font].key], k = size === 'huge' ? 2 : 1, w = s.length * f.w * k;
      let cx = align === 0 ? x - w / 2 : align === 1 ? x - w : x; cx = Math.round(cx);
      if (k === 1) { this.text(s, cx, y + FONTS[font].baseline, font, c); return; }
      const sheet = glyphSheet(font, c);
      for (let i = 0; i < s.length; i++) { let code = s.charCodeAt(i); if (code < 32 || code > 126) code = 32; if (code !== 32) this.g.drawImage(sheet, (code - 32) * f.w, 0, f.w, f.h, cx + i * f.w * k, y, f.w * k, f.h * k); }
    }
    textW(s, font) { return s.length * window.SPLEEN[FONTS[font].key].w; }
    textR(s, xRight, y, font, c) { this.text(s, xRight - this.textW(s, font), y, font, c); }
    // vector text in the Slint shell's faces (Space Grotesk / JetBrains Mono); y is the baseline
    ttf(str, x, y, size, color, o = {}) {
      const g = this.g; g.font = `${o.weight || 400} ${size}px ${o.family === 'sans' ? SANS : MONO}`; g.fillStyle = color;
      g.textAlign = o.align || 'left'; g.textBaseline = 'alphabetic';
      if ('letterSpacing' in g) g.letterSpacing = `${o.spacing || 0}px`;
      if (o.maxW) { while (str.length > 1 && g.measureText(str).width > o.maxW) str = str.slice(0, -1); }
      g.fillText(str, x, y);
      if ('letterSpacing' in g) g.letterSpacing = '0px';
    }
    ttfW(str, size, o = {}) { const g = this.g; g.font = `${o.weight || 400} ${size}px ${o.family === 'sans' ? SANS : MONO}`; if ('letterSpacing' in g) g.letterSpacing = `${o.spacing || 0}px`; const w = g.measureText(str).width; if ('letterSpacing' in g) g.letterSpacing = '0px'; return w; }
    rr(x, y, w, h, r, fill, stroke, lw = 1) {
      const g = this.g; g.beginPath(); g.roundRect(x, y, w, h, r);
      if (fill) { g.fillStyle = fill; g.fill(); }
      if (stroke) { g.strokeStyle = stroke; g.lineWidth = lw; g.beginPath(); g.roundRect(x + lw / 2, y + lw / 2, w - lw, h - lw, Math.max(0, r - lw / 2)); g.stroke(); }
    }
    arc(cx, cy, r, a0, a1, color, lw) { const g = this.g; g.strokeStyle = color; g.lineWidth = lw; g.lineCap = 'butt'; g.beginPath(); g.arc(cx, cy, r, a0, a1); g.stroke(); }
    image(img, x, y, w, h) { this.g.imageSmoothingEnabled = true; this.g.drawImage(img, x, y, w, h); this.g.imageSmoothingEnabled = false; }
    gradientV(x, y, w, h, c0, c1) {
      for (let i = 0; i < h; i++) this.rect(x, y + i, w, 1, blend(c0, c1, h > 1 ? i / (h - 1) : 0));
    }
  }

  window.ROVE = window.ROVE || {};
  Object.assign(window.ROVE, { FB, c565, blend, rgba, hexToRgb, rgbToHex, W, H });
})();
