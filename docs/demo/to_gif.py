# /// script
# dependencies = ["pillow==11.0.0"]
# ///
"""ghd.cast -> demo.gif: replay the recorded terminal into frames."""
import json, re, sys
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

d, fontdir, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
COLS, ROWS, CW, CH, PAD = 100, 30, 9, 18, 14
BG, FG, GREEN, CYAN, BAR = (15, 21, 18), (215, 226, 219), (111, 220, 155), (108, 199, 214), (26, 36, 32)
reg = ImageFont.truetype(str(fontdir / "JetBrainsMono-Regular.ttf"), 15)
bold = ImageFont.truetype(str(fontdir / "JetBrainsMono-Bold.ttf"), 15)
events = [json.loads(l) for l in (d / "ghd.cast").read_text().splitlines()[1:]]

lines = [[]]          # each line: list of (char, style)
style = (FG, False)
def feed(text):
    global style
    pos = 0
    for m in re.finditer(r"\x1b\[([0-9;]*)m", text + "\x1b[m"[:0]):
        put(text[pos:m.start()]); pos = m.end()
        codes = [c for c in m.group(1).split(";") if c]
        color, b = style
        if not codes or "0" in codes: color, b = FG, False
        for c in codes:
            if c == "1": b = True
            elif c == "32": color = GREEN
            elif c == "36": color = CYAN
        style = (color, b)
    put(text[pos:])
def put(s):
    for ch in s.replace("\r\n", "\n").replace("\r", ""):
        if ch == "\n": lines.append([])
        elif ch == "\t": lines[-1].extend([(" ", style)] * (8 - len(lines[-1]) % 8))
        elif len(lines[-1]) < COLS: lines[-1].append((ch, style))

W, H = COLS * CW + 2 * PAD, ROWS * CH + 2 * PAD + 26
def frame():
    img = Image.new("RGB", (W, H), BG); dr = ImageDraw.Draw(img)
    dr.rectangle([0, 0, W, 26], fill=BAR)
    for i, c in enumerate([(237, 106, 94), (245, 191, 79), (98, 197, 84)]):
        dr.ellipse([12 + i * 18, 8, 22 + i * 18, 18], fill=c)
    dr.text((W // 2 - 60, 5), "ghd demo — bash", font=reg, fill=(127, 143, 134))
    view = lines[-ROWS:]
    for r, line in enumerate(view):
        for c, (ch, (color, b)) in enumerate(line):
            if ch != " ":
                dr.text((PAD + c * CW, 26 + PAD + r * CH), ch, font=bold if b else reg, fill=color)
    r = len(view) - 1; c = len(view[-1])
    dr.rectangle([PAD + c * CW, 26 + PAD + r * CH + 2, PAD + c * CW + CW - 1, 26 + PAD + r * CH + CH - 2], fill=GREEN)
    return img

frames, durations, i, t_prev = [], [], 0, 0.0
STEP = 0.08
t = 0.0
end = events[-1][0] + 2.5
while t <= end:
    changed = False
    while i < len(events) and events[i][0] <= t:
        feed(events[i][2]); i += 1; changed = True
    if changed or not frames:
        frames.append(frame()); durations.append(STEP * 1000)
    else:
        durations[-1] += STEP * 1000
    t += STEP
durations[-1] = 4000  # hold the final screen
pal = [f.convert("P", palette=Image.ADAPTIVE, colors=32) for f in frames]
pal[0].save(out, save_all=True, append_images=pal[1:], duration=[int(x) for x in durations], loop=0, optimize=True, disposal=1)
print(len(frames), "frames", round(out.stat().st_size / 1024), "KiB", W, "x", H)
