/* Wires the handheld's DOM (pads, D-pad, stick, hand sensors, buttons, keyboard, screen pointer)
   to the device shell in os.js, and runs the frame loop. */
(() => {
  'use strict';
  const R = window.ROVE, A = R.audio;
  const $ = (s, r = document) => r.querySelector(s);
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));

  // Indiegogo links read one meta tag
  const meta = $('meta[name="indiegogo-url"]');
  if (meta && meta.content) document.querySelectorAll('a[data-igg]').forEach((a) => { a.href = meta.content; });

  const root = $('#rove'), fit = $('#fit'), canvas = $('#screen');
  if (!root || !window.SPLEEN) return;
  const os = new R.OS(canvas), raw = os.raw;
  const live = $('#live');
  os.say = (m) => { if (live) live.textContent = m; };
  document.fonts && document.fonts.ready && document.fonts.ready.then(() => {});

  /* ---------------- scale the 1460 x 640 design space to the page ---------------- */
  let k = 1;
  function resize() {
    if (!fit.clientWidth) return;   // hidden while a portrait phone is asked to turn
    k = fit.clientWidth / 1460;
    root.style.transform = `scale(${k})`; fit.style.height = `${640 * k}px`;
  }
  new ResizeObserver(resize).observe(fit); resize();

  /* ---------------- pads ---------------- */
  const PAD_KEYS = ['1', '2', '3', '4', 'q', 'w', 'e', 'r', 'a', 's', 'd', 'f', 'z', 'x', 'c', 'v'];
  const LED = { 1: '#5aa0ff', 2: '#ffd23c', 3: '#5cf07a', 4: '#ff5a5a' };
  const padsEl = $('#pads'), pads = [];
  for (let i = 0; i < 16; i++) {
    const b = document.createElement('button');
    b.type = 'button'; b.className = 'hh__pad'; b.setAttribute('aria-label', `Pad ${i + 1}`); b.setAttribute('aria-keyshortcuts', PAD_KEYS[i].toUpperCase());
    b.innerHTML = `<span aria-hidden="true">${i + 1}</span>`;
    padsEl.appendChild(b); pads.push(b);
    b.addEventListener('pointerdown', (e) => {
      if (e.pointerType === 'mouse' && e.button !== 0) return;
      e.preventDefault(); wake(); try { b.setPointerCapture(e.pointerId); } catch (_) { /* synthetic */ }
      setPad(i, true, e.pointerType === 'pen' || e.pointerType === 'touch' ? clamp(e.pressure || 1, 0.3, 1) : 1);
    });
    const up = () => setPad(i, false);
    b.addEventListener('pointerup', up); b.addEventListener('pointercancel', up); b.addEventListener('lostpointercapture', up);
    b.addEventListener('contextmenu', (e) => e.preventDefault());
    b.addEventListener('keydown', (e) => { if ((e.key === ' ' || e.key === 'Enter') && !e.repeat) { e.preventDefault(); e.stopPropagation(); setPad(i, true, 1, 'kbd'); } });
    b.addEventListener('keyup', (e) => { if (e.key === ' ' || e.key === 'Enter') setPad(i, false, 1, 'kbd'); });
  }
  const padSources = Array.from({ length: 16 }, () => new Set());
  function setPad(i, on, pressure = 1, src = 'ptr') {
    const s = padSources[i];
    if (on) s.add(src); else s.delete(src);
    raw.held[i] = s.size > 0; raw.pressure[i] = pressure;
    pads[i].toggleAttribute('data-on', raw.held[i]);
  }

  /* ---------------- audio wake: the browser needs a gesture ---------------- */
  function wake() { if (!A.ctx) { A.init(); } else if (A.ctx.state === 'suspended') A.ctx.resume(); }
  root.addEventListener('pointerdown', wake);

  /* ---------------- buttons ---------------- */
  const hold = (el, down, up) => {
    el.addEventListener('pointerdown', (e) => { if (e.pointerType === 'mouse' && e.button !== 0) return; e.preventDefault(); wake(); try { el.setPointerCapture(e.pointerId); } catch (_) { /* synthetic */ } el.setAttribute('data-down', ''); down(); });
    const end = () => { el.removeAttribute('data-down'); up(); };
    el.addEventListener('pointerup', end); el.addEventListener('pointercancel', end); el.addEventListener('lostpointercapture', end);
    el.addEventListener('keydown', (e) => { if ((e.key === 'Enter' || e.key === ' ') && !e.repeat) { e.preventDefault(); e.stopPropagation(); el.setAttribute('data-down', ''); down(); } });
    el.addEventListener('keyup', (e) => { if (e.key === 'Enter' || e.key === ' ') { el.removeAttribute('data-down'); up(); } });
  };
  document.querySelectorAll('.hh__f').forEach((b) => hold(b, () => { os.edges.f[+b.dataset.f]++; }, () => {}));
  document.querySelectorAll('.hh__dir').forEach((b) => hold(b, () => { raw.dir[b.dataset.dir] = true; }, () => { raw.dir[b.dataset.dir] = false; }));
  hold($('.hh__sel'), () => { raw.select = true; }, () => { raw.select = false; });
  document.querySelectorAll('.hh__shoulder').forEach((b) => {
    const side = b.dataset.btn;
    hold(b, () => { raw[side] = true; os.edges[side]++; }, () => { raw[side] = false; });
  });

  /* ---------------- stick ---------------- */
  const stick = $('#stick'), cap = $('.cap', stick);
  let sd = null;
  stick.addEventListener('pointerdown', (e) => { wake(); e.preventDefault(); stick.setPointerCapture(e.pointerId); sd = { x: e.clientX, y: e.clientY, moved: false }; });
  stick.addEventListener('pointermove', (e) => {
    if (!sd) return;
    const dx = (e.clientX - sd.x) / k, dy = (e.clientY - sd.y) / k;
    if (Math.abs(dx) > 4 || Math.abs(dy) > 4) sd.moved = true;
    setStick(clamp(dx / 26, -1, 1), clamp(dy / 26, -1, 1));
  });
  const stickEnd = () => { if (sd && !sd.moved) os.edges.stickClick++; sd = null; setStick(0, 0); };
  stick.addEventListener('pointerup', stickEnd); stick.addEventListener('pointercancel', stickEnd);
  function setStick(x, y) { raw.stick = [x, y]; cap.style.transform = `translate(${x * 18}px, ${y * 18}px)`; }

  /* ---------------- hand sensors ---------------- */
  document.querySelectorAll('.hh__hand').forEach((el) => {
    const i = +el.dataset.hand, bar = $('b', el); let sy = 0;
    const set = (v) => { raw.hands[i] = v; bar.style.width = `${44 * v}px`; };
    el.addEventListener('pointerdown', (e) => { wake(); e.preventDefault(); el.setPointerCapture(e.pointerId); sy = e.clientY; set(0.5); });
    el.addEventListener('pointermove', (e) => { if (el.hasPointerCapture(e.pointerId)) set(clamp(0.5 + (sy - e.clientY) / k / 80, 0.02, 1)); });
    const end = () => set(0);
    el.addEventListener('pointerup', end); el.addEventListener('pointercancel', end);
    el._set = set;
  });

  /* ---------------- the screen answers the mouse too (Bebot, Vistas, ...) ---------------- */
  const queuePointer = (p) => {
    const queue = os.pointerEvents, last = queue[queue.length - 1];
    if (p.type === 'move' && last && last.type === 'move') queue[queue.length - 1] = p;
    else queue.push(p);
  };
  const toScreen = (e) => { const r = canvas.getBoundingClientRect(); return { x: (((e.clientX - r.left) / r.width) * 800 - 16) / 1.2, y: ((e.clientY - r.top) / r.height) * 360 }; };
  canvas.addEventListener('pointerdown', (e) => { wake(); canvas.setPointerCapture(e.pointerId); const p = toScreen(e); raw.ptr = { ...p, down: true }; queuePointer({ ...p, type: 'down' }); });
  canvas.addEventListener('pointermove', (e) => { const p = toScreen(e); if (raw.ptr && raw.ptr.down) { raw.ptr = { ...p, down: true }; queuePointer({ ...p, type: 'move' }); } });
  const pend = (e) => { if (!raw.ptr) return; const p = toScreen(e); raw.ptr = null; queuePointer({ ...p, type: e.type === 'pointercancel' && os.mode === 'home' ? 'cancel' : 'up' }); };
  canvas.addEventListener('pointerup', pend); canvas.addEventListener('pointercancel', pend);
  canvas.addEventListener('wheel', (e) => { if (!armed) return; e.preventDefault(); const step = -Math.sign(e.deltaY); raw.wheel = step; os.nudgeFocused && os.nudgeFocused(step); }, { passive: false });

  /* ---------------- keyboard (only while the device is armed) ---------------- */
  let armed = false;
  const arm = (v) => { armed = v; root.classList.toggle('is-armed', v); };
  root.addEventListener('pointerdown', () => arm(true));
  root.addEventListener('focusin', () => arm(true));
  document.addEventListener('pointerdown', (e) => { if (!root.contains(e.target)) arm(false); });
  document.addEventListener('focusin', (e) => { if (!root.contains(e.target)) arm(false); });
  const DIRS = { ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right' };
  const FKEYS = { F1: 0, F2: 1, F3: 2, F4: 3 };
  const STICK_KEYS = { i: [0, -1], k: [0, 1], j: [-1, 0], l: [1, 0] };
  const stickHeld = new Set();
  const refreshStick = () => { let x = 0, y = 0; stickHeld.forEach((s) => { x += STICK_KEYS[s][0]; y += STICK_KEYS[s][1]; }); setStick(clamp(x, -1, 1), clamp(y, -1, 1)); };
  document.addEventListener('keydown', (e) => {
    if (!armed || e.ctrlKey || e.metaKey || e.altKey) return;
    const k1 = e.key.length === 1 ? e.key.toLowerCase() : e.key;
    if (e.key === 'Escape') { arm(false); root.blur(); releaseAllKeys(); return; }
    if (e.target.closest && e.target.closest('.hh__pad,.hh__f,.hh__dir,.hh__sel,.hh__shoulder') && (e.key === ' ' || e.key === 'Enter')) return;
    const pi = PAD_KEYS.indexOf(k1);
    if (pi >= 0) { e.preventDefault(); if (!e.repeat) setPad(pi, true, 1, 'key'); return; }
    if (DIRS[e.key]) { e.preventDefault(); raw.dir[DIRS[e.key]] = true; return; }
    if (e.key === 'Enter') { e.preventDefault(); raw.select = true; return; }
    if (e.key in FKEYS) { e.preventDefault(); if (!e.repeat) os.edges.f[FKEYS[e.key]]++; return; }
    if (k1 === '[') { e.preventDefault(); if (!raw.l1) os.edges.l1++; raw.l1 = true; return; }
    if (k1 === ']') { e.preventDefault(); if (!raw.r1) os.edges.r1++; raw.r1 = true; return; }
    if (k1 in STICK_KEYS) { e.preventDefault(); stickHeld.add(k1); refreshStick(); return; }
    if (k1 === ',' || k1 === '.') { e.preventDefault(); document.querySelectorAll('.hh__hand')[k1 === ',' ? 0 : 1]._set(0.7); }
  });
  document.addEventListener('keyup', (e) => {
    const k1 = e.key.length === 1 ? e.key.toLowerCase() : e.key, pi = PAD_KEYS.indexOf(k1);
    if (pi >= 0) setPad(pi, false, 1, 'key');
    if (DIRS[e.key]) raw.dir[DIRS[e.key]] = false;
    if (e.key === 'Enter') raw.select = false;
    if (k1 === '[') raw.l1 = false; if (k1 === ']') raw.r1 = false;
    if (k1 in STICK_KEYS) { stickHeld.delete(k1); refreshStick(); }
    if (k1 === ',' || k1 === '.') document.querySelectorAll('.hh__hand')[k1 === ',' ? 0 : 1]._set(0);
  });
  function releaseAllKeys() {
    for (let i = 0; i < 16; i++) { padSources[i].clear(); raw.held[i] = false; pads[i].removeAttribute('data-on'); }
    Object.keys(raw.dir).forEach((d) => { raw.dir[d] = false; }); raw.select = false; raw.l1 = raw.r1 = false; stickHeld.clear(); setStick(0, 0);
  }
  window.addEventListener('blur', releaseAllKeys);

  /* ---------------- volume, silence ---------------- */
  $('#vol').addEventListener('input', (e) => A.setVolume(e.target.value / 100));
  $('#silence').addEventListener('click', () => {
    os.apps.forEach((a) => { if (a.inst) { if (a.inst.running && a.inst.running() === true && a.inst.toggleRunning) a.inst.toggleRunning(); if (a.inst.leave && os.active === a) a.inst.leave(); } });
    A.demoLoop.stop(); os.say('All sound stopped.');
  });

  /* ---------------- the frame loop ---------------- */
  let visible = true, last = performance.now(), ledSig = '';
  new IntersectionObserver(([e]) => { visible = e.isIntersecting; }, { threshold: 0 }).observe(root);
  document.addEventListener('visibilitychange', () => { if (!A.ctx) return; if (document.hidden) A.ctx.suspend(); else A.ctx.resume(); });

  function frame(now) {
    const dt = Math.min(0.1, (now - last) / 1000); last = now;
    if (!document.hidden) {
      try {
        os.tick(dt);                       // generators keep ticking when the device is scrolled away; only drawing pauses
        if (visible) os.draw();
        const inst = os.active && os.active.inst, inApp = os.mode === 'app';
        const padMode = inst && inst.gridModeLabel ? inst.gridModeLabel() : '';
        const transport = inst && inst.transportAction ? inst.transportAction() : '';
        const labels = os.mode === 'home' ? ['HELP', 'CATEGORY', '—', 'MIXER'] : os.mode === 'mixer' ? ['HOME', '—', '—', 'BACK'] : ['HOME', inApp && padMode ? `PADS: ${padMode}` : '—', inApp && transport ? transport : '—', 'MIXER'];
        document.querySelectorAll('.hh__f').forEach((b, i) => {
          if (b.textContent !== labels[i]) b.textContent = labels[i];
          b.disabled = os.mode === 'boot' || (i === 1 && !(os.mode === 'home' || (inApp && padMode))) || (i === 2 && !(inApp && transport));
        });
        const leds = os.leds(), sig = leds.join('');
        if (sig !== ledSig) {
          ledSig = sig;
          leds.forEach((s, i) => { if (s) { pads[i].setAttribute('data-led', ''); pads[i].style.setProperty('--ledc', LED[s]); } else pads[i].removeAttribute('data-led'); });
        }
      } catch (err) { console.error('ROVE:', err); }
    }
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);

  R.os = os;
})();
