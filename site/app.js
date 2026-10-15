/* ROVE site: palette system + small page behaviours.
   The device demo lives in rove-demo.js. */
(() => {
  'use strict';

  // --- Indiegogo link: one place to change it is <meta name="indiegogo-url"> in index.html.
  const meta = document.querySelector('meta[name="indiegogo-url"]');
  if (meta && meta.content) {
    document.querySelectorAll('a[data-igg]').forEach((a) => { a.href = meta.content; });
  }

  // --- Palettes -------------------------------------------------------------
  // Colours are eyeballed from the 20-palette logo sheet, so treat the hex values as a first
  // pass and correct them against the source art. c = [tile 1..4], in logo order.
  // `ink` is the text colour: dark on light palettes, light on dark ones.
  const PALETTES = [
    { id: 'cobalt',   name: 'Cobalt Pop',      dark: false, bg: '#F4F3EC', ink: '#1F2140', c: ['#1F4FD8', '#F2692B', '#E3B81B', '#23264A'] },
    { id: 'sunset',   name: 'Sunset',          dark: false, bg: '#FDF3E7', ink: '#2A1F45', c: ['#F26B3A', '#D13B5C', '#7B3F86', '#2F3A7C'] },
    { id: 'lagoon',   name: 'Lagoon',          dark: false, bg: '#EAF7F4', ink: '#0B3B4A', c: ['#0F6377', '#1E9E9B', '#6CC9B4', '#E3B544'] },
    { id: 'bauhaus',  name: 'Bauhaus',         dark: false, bg: '#F6F2E8', ink: '#1A1A1A', c: ['#D02A2A', '#1F55B0', '#E0AE12', '#1A1A1A'] },
    { id: 'acid',     name: 'Acid',            dark: true,  bg: '#121212', ink: '#F2F5E0', c: ['#D8F000', '#9BD000', '#6A9A14', '#E8EFC0'] },
    { id: 'arctic',   name: 'Arctic',          dark: true,  bg: '#08213A', ink: '#EAF6FB', c: ['#5BC9E8', '#B8E6F4', '#7C96C8', '#F2FAFD'] },
    { id: 'ember',    name: 'Ember',           dark: true,  bg: '#1B1411', ink: '#F4E9D8', c: ['#FF5A1F', '#E8A33D', '#C4523A', '#D9C7A3'] },
    { id: 'candy',    name: 'Digital Candy',   dark: true,  bg: '#1B1535', ink: '#FFF8E8', c: ['#FF6FB0', '#B79CFF', '#7DE8E8', '#F5E27A'] },
  ];

  const hex2rgb = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
  const rgb2hex = (a) => '#' + a.map((v) => Math.round(v).toString(16).padStart(2, '0')).join('');
  const mix = (a, b, t) => { const x = hex2rgb(a), y = hex2rgb(b); return rgb2hex(x.map((v, i) => v + (y[i] - v) * t)); };
  const lin = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; };
  const lum = (h) => { const [r, g, b] = hex2rgb(h); return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b); };
  const ratio = (a, b) => { const x = lum(a), y = lum(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };

  // Text colour for a coloured block: prefer the palette's own ink/paper, fall back to white/black.
  function pickOn(fill, p) {
    const cands = [p.ink, p.bg, '#FFFFFF', '#111111'];
    const ok = cands.find((c) => ratio(fill, c) >= 4.5);
    return ok || cands.reduce((best, c) => (ratio(fill, c) > ratio(fill, best) ? c : best));
  }

  const root = document.documentElement;
  function applyPalette(p) {
    const s = (k, v) => root.style.setProperty(k, v);
    s('--bg', p.bg); s('--ink', p.ink);
    p.c.forEach((c, i) => { s(`--c${i + 1}`, c); s(`--on-c${i + 1}`, pickOn(c, p)); });
    const light = p.dark ? p.ink : p.bg;
    const screen = mix(p.dark ? p.bg : p.ink, '#000000', 0.5);
    s('--screen', screen);
    s('--screen-text', light);
    // accents for the device screen: a dark palette colour (e.g. the navy K tile) would vanish on the
    // near-black screen, so lift each one toward the light colour until it reaches 3:1
    p.c.forEach((c, i) => {
      let lifted = c;
      for (let t = 0.1; ratio(lifted, screen) < 3.2 && t <= 1; t += 0.1) lifted = mix(c, light, t);
      s(`--s${i + 1}`, lifted); s(`--on-s${i + 1}`, pickOn(lifted, p));
    });
    s('--deep', p.dark ? mix(p.bg, '#000000', 0.35) : p.ink);
    s('--deep-text', p.dark ? p.ink : p.bg);
    root.dataset.mode = p.dark ? 'dark' : 'light';
    root.dataset.palette = p.id;
    const tc = document.querySelector('meta[name="theme-color"]');
    if (tc) tc.content = p.bg;
    document.dispatchEvent(new CustomEvent('rove:palette', { detail: p }));
  }

  // --- picker UI ---
  const details = document.getElementById('palettes');
  const menu = details && details.querySelector('.palettes__menu');
  const summaryDots = details && details.querySelectorAll('summary .dots i');
  let current = PALETTES[0];

  const dotsHTML = (p) => `<span class="dots" aria-hidden="true">${p.c.map((c) => `<i style="background:${c}"></i>`).join('')}</span>`;

  function select(p, { persist = true } = {}) {
    current = p;
    applyPalette(p);
    if (summaryDots) summaryDots.forEach((d, i) => { d.style.background = p.c[i]; });
    if (menu) menu.querySelectorAll('button').forEach((b) => b.setAttribute('aria-pressed', String(b.dataset.id === p.id)));
    if (persist) { try { localStorage.setItem('rove-palette', p.id); } catch (_) { /* private mode */ } }
  }

  if (menu) {
    menu.innerHTML = '<p class="palettes__hint">Swap the whole site’s colours.</p>' +
      PALETTES.map((p) => `<button type="button" data-id="${p.id}" aria-pressed="false">${dotsHTML(p)}<span>${p.name}</span></button>`).join('');
    menu.addEventListener('click', (e) => {
      const b = e.target.closest('button'); if (!b) return;
      select(PALETTES.find((p) => p.id === b.dataset.id));
      details.open = false; details.querySelector('summary').focus();
    });
    document.addEventListener('click', (e) => { if (details.open && !details.contains(e.target)) details.open = false; });
    details.addEventListener('keydown', (e) => { if (e.key === 'Escape' && details.open) { details.open = false; details.querySelector('summary').focus(); } });
  }

  let saved = null;
  try { saved = localStorage.getItem('rove-palette'); } catch (_) { /* ignore */ }
  const params = new URLSearchParams(location.search).get('palette');
  select(PALETTES.find((p) => p.id === (params || saved)) || PALETTES[0], { persist: false });

  window.ROVE = Object.assign(window.ROVE || {}, { palettes: PALETTES, ratio, current: () => current });
})();
