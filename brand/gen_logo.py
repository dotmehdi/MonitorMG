import math, os, sys
OUT = sys.argv[1]
INK, AMBER, LIGHT = "#13294B", "#FFA300", "#F4F6FA"

def rr(x, y, w, h, r):  # rounded rect subpath, clockwise
    return (f"M{x+r} {y}H{x+w-r}A{r} {r} 0 0 1 {x+w} {y+r}V{y+h-r}A{r} {r} 0 0 1 {x+w-r} {y+h}"
            f"H{x+r}A{r} {r} 0 0 1 {x} {y+h-r}V{y+r}A{r} {r} 0 0 1 {x+r} {y}Z")

def mark(t=22, small=False):
    """Ring + knob. t = ring thickness (thinner for reversed)."""
    if small:  # pixel-snapped to a 16 px grid (1 px = 16 units)
        ring = rr(16, 48, 224, 160, 48) + rr(48, 80, 160, 96, 16)
        knob = rr(128, 96, 64, 64, 16)
    else:
        d = 22 - t
        ring = rr(24, 58, 208, 136, 36) + rr(24 + t, 58 + t, 208 - 2*t, 136 - 2*t, 36 - t)
        knob = rr(124 + d, 94 + d, 72 - d, 64 - 2*d, 22)
    return ring, knob

def symbol(ring_fill, knob_fill, **kw):
    ring, knob = mark(**kw)
    return (f'<path fill="{ring_fill}" fill-rule="evenodd" d="{ring}"/>'
            f'<path fill="{knob_fill}" d="{knob}"/>')

# --- wordmark: geometric monoline letters, baseline y=0, cap 100, x-height 74, stem 14
W, XH = 14, 74
def P(cx, cy, r, a):
    a = math.radians(a); return f"{cx + r*math.cos(a):.2f} {cy - r*math.sin(a):.2f}"
def L_M(x):
    return [rr_plain(x, -100, W, 100), rr_plain(x + 78, -100, W, 100),
            f"M{x} -100H{x+16}L{x+54} -24H{x+38}Z", f"M{x+76} -100H{x+92}L{x+54} -24H{x+38}Z"], 92
def rr_plain(x, y, w, h):
    return f"M{x} {y}H{x+w}V{y+h}H{x}Z"
def ring_c(cx, cy, R, r):
    return (f"M{cx-R} {cy}A{R} {R} 0 1 1 {cx+R} {cy}A{R} {R} 0 1 1 {cx-R} {cy}Z"
            f"M{cx-r} {cy}A{r} {r} 0 1 0 {cx+r} {cy}A{r} {r} 0 1 0 {cx-r} {cy}Z")
def L_o(x):
    return [ring_c(x + 38, -37, 38, 24)], 76
def L_n(x):
    return [rr_plain(x, -XH, W, XH),
            f"M{x} -42A32 32 0 0 1 {x+64} -42V0H{x+50}V-42A18 18 0 0 0 {x+14} -42Z"], 64
def L_i(x):
    return [rr_plain(x, -XH, W, XH), ring_c(x + 7, -92, 8.5, 0.001).split("Z")[0] + "Z"], W
def L_t(x):
    return [rr_plain(x + 10, -94, W, 94), rr_plain(x, -XH, 34, W)], 34
def L_r(x):
    return [rr_plain(x, -XH, W, XH),
            f"M{x} -42A32 32 0 0 1 {x+32} -74H{x+40}V-60H{x+32}A18 18 0 0 0 {x+14} -42Z"], 40
def L_G(x):
    cx, cy, R, r = x + 52, -50, 52, 38
    arc = (f"M{P(cx,cy,R,42)}A{R} {R} 0 1 0 {P(cx,cy,R,0)}L{P(cx,cy,r,0)}"
           f"A{r} {r} 0 1 1 {P(cx,cy,r,42)}Z")
    return [arc, rr_plain(cx + 4, cy - W, R - 4, W)], 104
GLYPH = {"M": L_M, "o": L_o, "n": L_n, "i": L_i, "t": L_t, "r": L_r, "G": L_G}
GAPS = [12, 11, 15, 12, 8, 11, 17, 12]  # optical spacing between consecutive letters

def wordmark():
    x, parts = 0, []
    for i, ch in enumerate("MonitorMG"):
        p, w = GLYPH[ch](x); parts += p; x += w + (GAPS[i] if i < len(GAPS) else 0)
    return "".join(f'<path fill-rule="evenodd" d="{d}"/>' for d in parts), x

WM, WM_W = wordmark()

def svg(vb, body, title="MonitorMG"):
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{vb}"><title>{title}</title>{body}</svg>\n'

def write(name, content):
    open(os.path.join(OUT, name), "w", encoding="utf-8").write(content)

themes = {"": (INK, AMBER, INK, 22), "-reversed": (LIGHT, AMBER, LIGHT, 20),
          "-black": ("#000000", "#000000", "#000000", 22), "-white": ("#FFFFFF", "#FFFFFF", "#FFFFFF", 20)}
s = 0.84  # wordmark scale: cap height 84 vs ring height 136
for suf, (rf, kf, tf, t) in themes.items():
    write(f"monitormg-symbol{suf}.svg", svg("0 0 256 256", symbol(rf, kf, t=t)))
    # horizontal: ring centre y=126, wordmark cap centred on it
    gx = 280; w = int(gx + WM_W * s + 24)
    body = symbol(rf, kf, t=t) + f'<g fill="{tf}" transform="translate({gx} {126 + 42}) scale({s})">{WM}</g>'
    write(f"monitormg-horizontal{suf}.svg", svg(f"0 0 {w} 256", body))
    # stacked: symbol on top, wordmark centred below
    sw = WM_W * s; W2 = int(max(256, sw) + 48); sx = (W2 - 256) / 2
    body = (f'<g transform="translate({sx} 0)">{symbol(rf, kf, t=t)}</g>'
            f'<g fill="{tf}" transform="translate({(W2 - sw)/2:.1f} {194 + 40 + 84}) scale({s})">{WM}</g>')
    write(f"monitormg-stacked{suf}.svg", svg(f"0 0 {W2} {194 + 40 + 84 + 40}", body))
write("monitormg-wordmark.svg", svg(f"-4 -104 {WM_W + 8} 108", f'<g fill="{INK}">{WM}</g>'))
write("monitormg-symbol-small.svg", svg("0 0 256 256", symbol(INK, AMBER, small=True)))
write("monitormg-symbol-small-white.svg", svg("0 0 256 256", symbol("#FFFFFF", AMBER, small=True)))
print("wordmark width", WM_W)
