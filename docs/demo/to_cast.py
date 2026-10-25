"""script(1) timing+typescript -> asciicast v2, with typed-out commands."""
import json, sys
from pathlib import Path
d = Path(sys.argv[1])
raw = (d / "session.log").read_bytes()
raw = raw[raw.index(b"\n") + 1:]  # drop "Script started ..." header
events, t, pos = [], 0.0, 0
for line in (d / "timing.log").read_text().split("\n"):
    if not line.strip():
        continue
    delay, n = line.split()
    t += min(float(delay), 1.2)          # cap idle gaps; `time` shows real durations
    chunk = raw[pos:pos + int(n)]; pos += int(n)
    events.append([t, chunk])
out, shift = [], 0.0
for t, chunk in events:
    text = chunk.decode("utf-8", "replace").replace("\n", "\r\n").replace("\r\r\n", "\r\n")
    while "\x01" in text:
        pre, rest = text.split("\x01", 1)
        typed, text = rest.split("\x02", 1)
        if pre:
            out.append([round(t + shift, 3), pre])
        shift += 0.35
        for ch in typed:
            shift += 0.022
            out.append([round(t + shift, 3), ch])
        shift += 0.25
    if text:
        out.append([round(t + shift, 3), text])
header = {"version": 2, "width": 100, "height": 34, "title": "ghd demo", "env": {"TERM": "xterm-256color"}}
cast = json.dumps(header) + "\n" + "\n".join(json.dumps([e[0], "o", e[1]]) for e in out) + "\n"
(d / "ghd.cast").write_text(cast)
print(f"{len(out)} events, {out[-1][0]:.1f}s")
