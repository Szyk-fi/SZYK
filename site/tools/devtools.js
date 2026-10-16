/* dev helpers, pasted into the console while testing: eval(await (await fetch('/tools/devtools.js')).text()) */
window.peek = () => { let d = document.getElementById('peek'); if (!d) { d = document.createElement('div'); d.id = 'peek'; d.style.cssText = 'position:fixed;left:0;top:0;width:800px;height:400px;background:#222;z-index:99999'; d.innerHTML = '<img style="position:absolute;left:80px;top:20px;width:640px;height:360px;image-rendering:pixelated">'; document.body.appendChild(d); } d.querySelector('img').src = document.getElementById('screen').toDataURL(); };
window.shot = async (name, pre, wait = 700) => { document.documentElement.style.scrollBehavior = 'auto'; window.scrollTo(0, document.getElementById('fit').getBoundingClientRect().top + scrollY - 5); const a = ROVE.os.apps.find((x) => x.def.id === name); ROVE.os.open(a); if (pre) await pre(a.inst); await new Promise((r) => setTimeout(r, wait)); peek(); return a.inst.view; };
window.testApp = async (id, pads = [12, 15, 5], secs = 3) => {
  const A = ROVE.audio, os = ROVE.os, SR = 44100;
  window.AudioContext = function () { return new OfflineAudioContext(2, SR * secs, SR); };
  A.ctx = null; A.channels.clear(); const a = os.apps.find((x) => x.def.id === id); a.inst = null; os.active = null;
  A.init(); os.open(a); const inst = a.inst, spec = inst.spec; let err = null;
  try { pads.forEach((p, i) => { const v = spec.noteOn(inst, p, 0.9, 0.1 + i * 0.6); if (v) { inst.voices.set(p, v); v.stop && v.stop(0.1 + i * 0.6 + 0.5); } }); } catch (e) { err = String(e.stack || e).split('\n').slice(0, 3).join(' | '); }
  const buf = await A.ctx.startRendering(), d = buf.getChannelData(0); let pk = 0, s = 0, bad = 0;
  for (const v of d) { if (!isFinite(v)) bad++; pk = Math.max(pk, Math.abs(v)); s += v * v; }
  return { id, err, peak: +pk.toFixed(3), rms: +Math.sqrt(s / d.length).toFixed(4), bad };
};
/* realtime test for generators and effects: open, start, listen on the channel meter for `ms` */
window.liveTest = async (id, ms = 3000) => {
  const A = ROVE.audio, os = ROVE.os; const a = os.apps.find((x) => x.def.id === id); A.init(); await A.ctx.resume(); os.open(a);
  const inst = a.inst; let err = null; try { if (inst.running && inst.running() === false && inst.toggleRunning) inst.toggleRunning(); } catch (e) { err = String(e.stack || e).split('\n').slice(0, 3).join(' | '); }
  let pk = 0; const t0 = performance.now(); while (performance.now() - t0 < ms) { await new Promise((r) => setTimeout(r, 100)); pk = Math.max(pk, inst.ch.rms()); }
  const state = A.ctx.state; if (inst.running && inst.running() === true && inst.toggleRunning && id !== 'tonestack') { try { inst.toggleRunning(); } catch (_) { /* ok */ } }
  os.home(); return { id, err, state, rms: +pk.toFixed(4) };
};
