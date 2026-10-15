# ROVE / SZYK website

Static site, no build step: `index.html`, `styles.css`, `app.js` (palettes), `rove-demo.js` (the playable device).

```sh
cd /Users/max/Developer/Modulade/portamax-sim && python3 -m http.server 8765 --directory site   # then open http://localhost:8765
```

Deploys as-is to any static host (GitHub Pages, Netlify, Cloudflare Pages). The only external request is Google Fonts (Rubik, JetBrains Mono); system fonts take over if it is blocked.

## Things to fill in before launch

- **Indiegogo URL**: `<meta name="indiegogo-url">` in `index.html` (every "Back it" button reads it). It is a placeholder.
- **Palette**: Cobalt Pop is the default. The hex values in `app.js` (`PALETTES`) were eyeballed from the logo sheet, so check them against the source art. `?palette=ember` in the URL forces one; the header picker lets visitors choose.
- **Numbers**: 116 apps / 31 instruments / 21 effects / 15 sequencers come from `apps/*/manifest.toml` as of writing.
- **Social card**: no `og:image` yet (needs a PNG, ~1200×630).
- **Copy to confirm**: the FAQ's status answer (software runs in a simulator, hardware is STM32N6-based), the two I/O card slots, and the Mutable Instruments credit/non-affiliation line.

## The demo

Five Web Audio voices, not the native engines, and the page says so: Sub Bass (saw + square sub through a resonant low-pass), Glass FM (2-operator FM), String (Karplus-Strong), Blaster (hold to charge, release to fire; the idea of the real Blaster app), Drums (synthesised kit, six-square metal hats). All pads play A minor pentatonic so nothing sounds wrong.

Controls: tap or click pads; drag knobs (Shift = fine, wheel, double-click resets); D-pad left/right = instrument, up/down = octave; F1 demo loop, F2 echo/room, F3 randomise knobs, F4 reset. Computer keys `1234 QWER ASDF ZXCV`, `[ ]` and `, .` for the knobs work only while the device is focused or was just clicked, so they never capture keys from the rest of the page.

Accessibility: every control is a real button or `role="slider"`, visible focus rings, `aria-live` announcements, reduced-motion respected, all palettes checked for 4.5:1 text contrast. The demo loop stops by itself when the device scrolls out of view or the tab is hidden.
