import json, re, sys
from pathlib import Path
d = Path(sys.argv[1])
lines = (d / "ghd.cast").read_text().splitlines()
events = [[e[0], e[2]] for e in map(json.loads, lines[1:])]
chapters, buf_t = [], None
for t, s in events:
    for m in re.finditer(r"\x1b\[1;36m# (.*?)\x1b\[0m", s):
        chapters.append([round(t, 1), m.group(1)])
payload = json.dumps({"events": events, "chapters": chapters}).replace("</", "<\\/")
html = (d / "page.html").read_text().replace("__CAST__", payload)
(d / "ghd-demo.html").write_text(html)
print(len(chapters), "chapters;", round(len(html) / 1024), "KiB")
