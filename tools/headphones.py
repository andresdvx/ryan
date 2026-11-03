"""Genera los auriculares en pixel art (32x24, ampliados x4) de cada personaje, con el color de su "accent".

Uso: python tools/headphones.py   (necesita Pillow). Escribe assets/headphones/<id>.png.
La posición sobre la cabeza se ajusta con "headphones" en assets/characters.json.
"""
import json, os, sys
from PIL import Image, ImageDraw

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "assets", "headphones")
os.makedirs(OUT, exist_ok=True)
W, H, SCALE = 32, 24, 4
OUTLINE = (30, 27, 40, 255)
CUSHION = (58, 54, 72, 255)


def shade(rgb, k):
    return tuple(max(0, min(255, int(c * k))) for c in rgb) + (255,)


def hexrgb(h):
    h = h.lstrip("#")
    return tuple(int(h[i:i + 2], 16) for i in (0, 2, 4))


def draw(accent):
    main = tuple(accent) + (255,)
    light = tuple(min(255, int(c * 0.55 + 255 * 0.45)) for c in accent) + (255,)
    dark = shade(accent, 0.72)
    im = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    px = im.load()
    cx, cy, rx, ry = 15.5, 15.0, 14.0, 13.0
    # diadema: un arco de 2-3 px con contorno
    for y in range(0, 12):
        for x in range(W):
            e = ((x + 0.5 - cx) / rx) ** 2 + ((y + 0.5 - cy) / ry) ** 2
            if 0.70 <= e <= 1.0:
                px[x, y] = main
                if e <= 0.78:
                    px[x, y] = dark
                elif e >= 0.93 and x < 16 and y < 6:
                    px[x, y] = light
            elif 1.0 < e <= 1.20 or 0.56 <= e < 0.70:
                px[x, y] = OUTLINE

    def cup(x0, x1):
        for y in range(9, 21):
            for x in range(x0, x1 + 1):
                edge = x in (x0, x1) or y in (9, 20)
                corner = (x in (x0, x1)) and (y in (9, 20))
                if corner:
                    continue
                px[x, y] = OUTLINE if edge else main
        for y in range(11, 19):                      # brillo vertical
            px[x0 + 1, y] = light
        for y in range(11, 19):                      # sombra del lado opuesto
            px[x1 - 1, y] = dark

    cup(1, 6)
    cup(25, 30)
    # almohadilla oscura (1 px) en el lado que mira a la cabeza
    for y in range(11, 19):
        px[7, y] = CUSHION
        px[24, y] = CUSHION
    return im.resize((W * SCALE, H * SCALE), Image.NEAREST)


if __name__ == "__main__":
    data = json.load(open(os.path.join(ROOT, "assets", "characters.json"), encoding="utf-8"))
    for c in data["characters"]:
        draw(hexrgb(c["accent"])).save(os.path.join(OUT, f"{c['id']}.png"))
    print("ok", os.listdir(OUT))
