"""Rewrites the app <script> block in index.html from the files present in js/data and js/apps."""
import re, pathlib
root = pathlib.Path(__file__).resolve().parents[1]
order = ['_effect','voltage','atlas','hydra','dexed','tinkertone','blaster','swarm','chordsmith','june60','bebot','vistas','skins','pulsar','sequencer','bloom','turing','squeeze','prism','vectorfilter','tonestack']
lines = []
for d in sorted((root/'js/data').glob('*.js')): lines.append(f'<script src="js/data/{d.name}"></script>')
for n in order:
    if (root/f'js/apps/{n}.js').exists(): lines.append(f'<script src="js/apps/{n}.js"></script>')
html = (root/'index.html').read_text()
html = re.sub(r'<!-- apps:begin -->.*?<!-- apps:end -->', '<!-- apps:begin -->\n' + '\n'.join(lines) + '\n<!-- apps:end -->', html, flags=re.S)
import time
v = str(int(time.time()))
html = re.sub(r'(src="js/[^"?]+)(\?v=\d+)?"', lambda m: m.group(1) + '?v=' + v + '"', html)   # cache-bust while developing
html = re.sub(r'(href="styles\.css)(\?v=\d+)?"', lambda m: m.group(1) + '?v=' + v + '"', html)
(root/'index.html').write_text(html)
print(len(lines), 'scripts')
