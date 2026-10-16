# ROVE / SZYK website

Static site, no build step: `index.html`, `styles.css`, and `js/` (the handheld simulator and its 20 apps).

```sh
cd /Users/max/Developer/Modulade/portamax-sim && python3 -m http.server 8765 --directory site   # then open http://localhost:8765
```

Deploys as-is to any static host (GitHub Pages, Netlify, Cloudflare Pages). The only external request is Google Fonts (Space Grotesk, JetBrains Mono); system fonts take over if it is blocked.

## Things to fill in before launch

- **Indiegogo URL**: `<meta name="indiegogo-url">` in `index.html` (every "Back it" button reads it). It is a placeholder.
- **Expansion cards**: the copy says four card slots, chosen at purchase. The repo's hardware docs (`docs/IO_CARDS.md`, `src/io_cards.rs`) currently describe two slots, so confirm and align them.
- **Camera**: drawn above pad 1 on the site and in `examples/slint_common/device_frame.slint`. Nothing in the software uses it yet.
- **Community and open source**: the page promises a community to post, rate and review apps (future tense) and says the whole system is open source. ROVE's own code is MIT (`LICENSE`); bundled third-party code keeps its own licences.
- **Social card**: no `og:image` yet (needs a PNG, about 1200x630).

## The handheld

The frame is a replica of `examples/slint_common/device_frame.slint`. The 640x360 screen is a canvas:

- 17 apps use the Slint shell look (title row, shared play column, right-hand card, hint line, F-key bar), measured from the Slint renders (`slint.js`, `playkit.js`).
- Chordsmith, Bebot and Skins draw their own full screens with the firmware's Spleen bitmap font, as the real apps do (`fb.js`, `spleen.js`). Bebot has no robot.
- Real data is used where the repo has it: Voltage's 20 factory presets, Atlas's 7 presets and macro targets, Tinkertone's 22 tones and 6 rhythms, and 32 DX7 voices with the DX7 algorithm table for Dexed.
- The sound engines are Web Audio versions written for the page, and the page says so. Do not claim they are the native engines.

Controls: pads, D-pad, select, stick, hand sensors, F1-F4, L1/R1, or the keyboard once the device is focused (digits and letters for pads, arrows, Enter, F1-F4, I J K L for the stick, `[` `]`). Bebot and the screen also answer the mouse.

## Developing

- `python3 site/tools/wire_apps.py` rewrites the app `<script>` list and cache-busts local files.
- `site/tools/devtools.js` has console helpers (`shot`, `testApp`, `liveTest`).
- `site/tools/make_spleen.py` regenerates `js/spleen.js` from `fonts/spleen_src` (BSD-2-Clause, see `assets/LICENSE-spleen`).
