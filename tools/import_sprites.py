#!/usr/bin/env python3
"""Importa una hoja de sprites (varios frames en una imagen) como la animación de UN estado de un personaje.

Corta la hoja en frames, quita el fondo, los ajusta al tamaño y a la "línea del suelo" del personaje, los guarda en
assets/<carpeta del personaje>/<estado>/ y declara el estado en assets/characters.json. Después basta con recompilar.

Ejemplos (desde la raíz del proyecto; necesita Pillow: pip install pillow):

    python tools/import_sprites.py peter_panda walking hoja.png --cols 6
    python tools/import_sprites.py peter_panda sleeping hoja.png --cols 4 --bg "#00ff00"
    python tools/import_sprites.py pingu happy hoja.png --cols 3 --rows 2 --preview vista.png
    python tools/import_sprites.py peter_panda walking --remove      # vuelve a la animación de siempre
    python tools/import_sprites.py --list                            # qué estados tiene cada personaje

Estados: idle, walking, thinking, working, success, happy, surprised, interacting, alert, sleeping, listening.
La hoja es una cuadrícula de celdas iguales, leída de izquierda a derecha y de arriba abajo (--cols x --rows).
"""
import argparse
import json
import os
import shutil
import statistics
import sys

from PIL import Image, ImageChops, ImageFilter

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# Cuánto dura una vuelta completa de cada animación, en milisegundos. De aquí sale el ritmo (ms por frame) cuando no
# se indica --ms: un paseo de 6 frames a 150 ms, un reposo de 4 frames a 450 ms, etc.
CYCLE_MS = {"idle": 1800, "walking": 900, "thinking": 1500, "working": 900, "success": 900, "happy": 1100, "surprised": 600,
            "interacting": 1000, "alert": 700, "sleeping": 3600, "listening": 1500}
STATES = ["idle", "walking", "thinking", "working", "success", "happy", "surprised", "interacting", "alert", "sleeping", "listening"]


# ---------------------------------------------------------------------------------------------
# Manifiesto: se escribe con el mismo formato que a mano, para que git solo vea lo que cambia
# ---------------------------------------------------------------------------------------------

def inline(value):
    if isinstance(value, dict):
        return "{ " + ", ".join(f"{json.dumps(k, ensure_ascii=False)}: {inline(v)}" for k, v in value.items()) + " }"
    if isinstance(value, list):
        return "[" + ", ".join(inline(v) for v in value) + "]"
    return json.dumps(value, ensure_ascii=False)


def dump_manifest(data):
    lines = []
    for key, value in data.items():
        if key != "characters":
            lines.append(f"  {json.dumps(key, ensure_ascii=False)}: {inline(value)}")
            continue
        blocks = []
        for char in value:
            fields = []
            for k, v in char.items():
                if k == "states":
                    if v:
                        rows = [f"        {json.dumps(s)}: {inline(spec)}" for s, spec in v.items()]
                        fields.append('      "states": {\n' + ",\n".join(rows) + "\n      }")
                else:
                    fields.append(f"      {json.dumps(k)}: {inline(v)}")
            blocks.append("    {\n" + ",\n".join(fields) + "\n    }")
        lines.append('  "characters": [\n' + ",\n".join(blocks) + "\n  ]")
    return "{\n" + ",\n".join(lines) + "\n}\n"


def read_manifest(path):
    return json.load(open(path, encoding="utf-8"))


def write_manifest(path, data):
    text = dump_manifest(data)
    json.loads(text)  # nunca se deja un manifiesto roto
    newline = "\r\n" if b"\r\n" in open(path, "rb").read() else "\n"  # se respeta el salto de línea que ya tenía
    with open(path, "wb") as out:
        out.write(text.replace("\n", newline).encode("utf-8"))


# ---------------------------------------------------------------------------------------------
# Imagen
# ---------------------------------------------------------------------------------------------

def parse_color(text):
    text = text.lstrip("#")
    if len(text) != 6 or any(c not in "0123456789abcdefABCDEF" for c in text):
        sys.exit(f"Color no válido: «{text}». Usa el formato #rrggbb (por ejemplo #00ff00).")
    return tuple(int(text[i:i + 2], 16) for i in (0, 2, 4))


def border_color(rgb):
    """El color más repetido en el borde de la imagen (agrupando tonos casi iguales): casi siempre, el del fondo."""
    w, h = rgb.size
    px = rgb.load()
    counts = {}
    edge = [(x, y) for x in range(w) for y in (0, h - 1)] + [(x, y) for y in range(h) for x in (0, w - 1)]
    for x, y in edge:
        r, g, b = px[x, y]
        key = (r >> 4, g >> 4, b >> 4)
        counts[key] = counts.get(key, 0) + 1
    r, g, b = max(counts, key=counts.get)
    return (r << 4 | 8, g << 4 | 8, b << 4 | 8)


def chroma_channel(color):
    """Si el fondo es un verde, rojo o azul «de croma» (como #00ff00), devuelve su canal (0, 1 o 2); si no, None."""
    k = max(range(3), key=lambda i: color[i])
    others = [color[i] for i in range(3) if i != k]
    return k if color[k] >= 150 and max(others) <= color[k] - 100 else None


def key_out(img, bg, tol, trim=1):
    """Devuelve la imagen con el fondo transparente. «auto»: si ya hay transparencia no toca nada; si no, usa el color
    más repetido en el borde. Un fondo de croma (verde puro, por ejemplo) se quita por «dominancia» del canal, que
    borra también los puntos sueltos y los bordes mezclados, y se limpia el tinte verde que queda en el contorno."""
    img = img.convert("RGBA")
    if bg == "none":
        return img
    if bg == "auto" and img.getchannel("A").getextrema()[0] < 250:
        return img
    rgb = img.convert("RGB")
    color = border_color(rgb) if bg == "auto" else parse_color(bg)
    k = chroma_channel(color)
    if k is not None:
        chans = list(rgb.split())
        top_other = ImageChops.lighter(*[chans[i] for i in range(3) if i != k])
        dominance = ImageChops.subtract(chans[k], top_other)
        lo, hi = max(4, tol // 2), tol + 24
        keyed = dominance.point(lambda v: 255 if v <= lo else (0 if v >= hi else int(255 * (hi - v) / (hi - lo))))
        chans[k] = ImageChops.darker(chans[k], top_other)  # quita el tinte del croma en el contorno
        rgb = Image.merge("RGB", chans)
    else:
        diff = ImageChops.difference(rgb, Image.new("RGB", rgb.size, color))
        r, g, b = (channel.point(lambda v: 255 if v <= tol else 0) for channel in diff.split())
        keyed = ImageChops.multiply(ImageChops.multiply(r, g), b)
    alpha = ImageChops.multiply(img.getchannel("A"), keyed)
    for _ in range(trim):  # se come el borde: ahí queda el color del fondo mezclado con el del personaje
        alpha = alpha.filter(ImageFilter.MinFilter(3))
    out = rgb.convert("RGBA")
    out.putalpha(alpha)
    return out


def slice_sheet(sheet, cols, rows, frames):
    """Cuadrícula de celdas iguales (--split grid)."""
    cw, ch = sheet.width // cols, sheet.height // rows
    if cw < 8 or ch < 8:
        sys.exit(f"Las celdas salen de {cw}x{ch} px: revisa --cols y --rows.")
    cells = [sheet.crop((c * cw, r * ch, (c + 1) * cw, (r + 1) * ch)) for r in range(rows) for c in range(cols)]
    if frames > len(cells):
        sys.exit(f"Pides {frames} frames pero la cuadrícula {cols}x{rows} solo tiene {len(cells)} celdas.")
    return cells[:frames]


def label_components(mask):
    """Regiones conectadas de una máscara 0/255: (etiquetas por píxel, cuántas, cajas, áreas). Pocos píxeles: va en Python puro."""
    w, h = mask.size
    data = mask.tobytes()
    lab = [0] * (w * h)
    boxes, areas, count = [None], [0], 0
    for start in range(w * h):
        if not data[start] or lab[start]:
            continue
        count += 1
        lab[start] = count
        stack = [start]
        x0 = x1 = start % w
        y0 = y1 = start // w
        area = 0
        while stack:
            p = stack.pop()
            x, y = p % w, p // w
            area += 1
            x0, x1, y0, y1 = min(x0, x), max(x1, x), min(y0, y), max(y1, y)
            for ny in range(max(0, y - 1), min(h, y + 2)):
                for nx in range(max(0, x - 1), min(w, x + 2)):
                    q = ny * w + nx
                    if data[q] and not lab[q]:
                        lab[q] = count
                        stack.append(q)
        boxes.append((x0, y0, x1 + 1, y1 + 1))
        areas.append(area)
    return lab, count, boxes, areas


def split_row(row, n):
    """Encuentra los n personajes de una fila de la hoja sin suponer celdas iguales: detecta cada figura por su silueta;
    si dos se tocan (la bufanda de uno con el siguiente) las separa por el hueco más estrecho entre ellas."""
    alpha = row.getchannel("A")
    s = max(1, -(-row.width // 640))  # se analiza a menor tamaño: basta para saber qué es de quién
    solid = alpha.point(lambda v: 255 if v > 40 else 0)
    small = solid.reduce(s).point(lambda v: 255 if v > 0 else 0) if s > 1 else solid
    small = small.filter(ImageFilter.MinFilter(3)).filter(ImageFilter.MaxFilter(3))  # «apertura»: borra líneas y motas de 1-2 px
    small = small.filter(ImageFilter.MaxFilter(5))  # y une los trocitos de una misma figura
    w, h = small.size
    lab, count, boxes, areas = label_components(small)
    if count == 0:
        sys.exit("No encuentro ningún personaje en la hoja: ¿el fondo se quitó entero? Prueba con otro --bg.")
    biggest = max(areas[1:])
    groups = [{"ids": {i}, "xr": (0, w)} for i in sorted((i for i in range(1, count + 1) if areas[i] >= 0.02 * biggest), key=lambda i: boxes[i][0])]

    def span(group):
        xs = [(boxes[i][0], boxes[i][2]) for i in group["ids"]]
        return max(min(a for a, _ in xs), group["xr"][0]), min(max(b for _, b in xs), group["xr"][1])

    while len(groups) > n:  # sobran trozos (una cola suelta, por ejemplo): se une la pareja más cercana
        gaps = [span(groups[i + 1])[0] - span(groups[i])[1] for i in range(len(groups) - 1)]
        i = gaps.index(min(gaps))
        groups[i] = {"ids": groups[i]["ids"] | groups[i + 1]["ids"], "xr": (0, w)}
        del groups[i + 1]
    widths = sorted(span(g)[1] - span(g)[0] for g in groups)
    single = widths[len(widths) // 2] if len(groups) > 1 else (span(groups[0])[1] - span(groups[0])[0]) / n
    while len(groups) < n:  # faltan figuras: alguna está pegada a la siguiente; se corta por el hueco más estrecho
        i = max(range(len(groups)), key=lambda j: span(groups[j])[1] - span(groups[j])[0])
        g = groups[i]
        x0, x1 = span(g)
        if x1 - x0 < 1.3 * single:
            sys.exit(f"Solo encuentro {len(groups)} figuras y esperaba {n}: revisa --cols o prueba --split grid.")
        ideal = x0 + single
        window = range(max(x0 + 1, int(ideal - 0.4 * single)), min(x1 - 1, int(ideal + 0.4 * single)) + 1)
        cost = {x: sum(1 for y in range(h) if lab[y * w + x] in g["ids"]) + abs(x - ideal) * 0.05 for x in window}
        cut = min(cost, key=cost.get)
        groups[i:i + 1] = [{"ids": g["ids"], "xr": (g["xr"][0], cut)}, {"ids": g["ids"], "xr": (cut, g["xr"][1])}]
    frames = []
    for g in groups:
        a, b = g["xr"]
        owned = Image.frombytes("L", (w, h), bytes(255 if lab[p] in g["ids"] and a <= p % w < b else 0 for p in range(w * h)))
        owned = owned.resize(row.size, Image.NEAREST)
        if s > 1:
            owned = owned.filter(ImageFilter.MaxFilter(2 * s + 1))
        frame = row.copy()
        frame.putalpha(ImageChops.multiply(alpha, owned))
        box = frame.getchannel("A").getbbox()
        if box:
            frames.append(frame.crop(box))
    return frames


def extract_frames(sheet, cols, rows, frames, split):
    if split == "grid":
        return slice_sheet(sheet, cols, rows, frames)
    out = []
    rh = sheet.height // rows
    for r in range(rows):
        out.extend(split_row(sheet.crop((0, r * rh, sheet.width, (r + 1) * rh)), cols))
    if frames > len(out):
        sys.exit(f"Pides {frames} frames pero solo encuentro {len(out)}.")
    return out[:frames]


def reference_box(path):
    """Tamaño del lienzo y caja del personaje en su sprite de referencia: de ahí salen la escala y el suelo."""
    ref = Image.open(path).convert("RGBA")
    box = ref.getchannel("A").getbbox()
    if not box:
        sys.exit(f"El sprite de referencia {path} está vacío.")
    return ref.size, box


def fit_frames(cells, canvas, ref_box, resample, boost=1.0):
    """Misma escala para todos los frames (la mediana de sus alturas iguala a la del personaje), apoyados en el mismo
    suelo y centrados en horizontal. Si algún frame no cabe, se reduce la escala de todos a la vez."""
    boxes = []
    for cell in cells:
        box = cell.getchannel("A").getbbox()
        if not box:
            sys.exit("Hay un frame vacío tras quitar el fondo: prueba otro --bg o sube --tol.")
        boxes.append(box)
    heights = [b[3] - b[1] for b in boxes]
    ref_h = ref_box[3] - ref_box[1]
    widths = [b[2] - b[0] for b in boxes]
    wanted = ref_h / statistics.median(heights) * boost
    margin = max(2, canvas[0] // 96)
    scale = min(wanted, (canvas[0] - 2 * margin) / max(widths), (canvas[1] - 2 * margin) / max(heights))
    if scale < 0.85 * wanted:
        print(f"AVISO: los frames se han reducido a un {scale / wanted:.0%} para que quepa el más grande. Suele ser un resto en la hoja "
              f"(una línea o una mota) o dos personajes pegados que no se separaron. Mira la vista previa y, si hace falta, usa --split grid.")
    if max(widths) > 1.8 * statistics.median(widths):
        print("AVISO: hay un frame mucho más ancho que los demás: puede traer restos del vecino.")
    ground = min(ref_box[3], canvas[1] - margin)
    out = []
    for cell, box in zip(cells, boxes):
        piece = cell.crop(box)
        size = (max(1, round(piece.width * scale)), max(1, round(piece.height * scale)))
        piece = piece.resize(size, resample)
        frame = Image.new("RGBA", canvas, (0, 0, 0, 0))
        frame.alpha_composite(piece, (round((canvas[0] - size[0]) / 2), max(0, ground - size[1])))
        out.append(frame)
    return out


def contact_sheet(frames, path):
    cols = min(len(frames), 6)
    rows = -(-len(frames) // cols)
    w, h = frames[0].size
    scale = min(1.0, 256 / w)
    tw, th = int(w * scale), int(h * scale)
    sheet = Image.new("RGBA", (cols * tw, rows * th), (64, 56, 70, 255))
    for i, frame in enumerate(frames):
        sheet.alpha_composite(frame.resize((tw, th)), ((i % cols) * tw, (i // cols) * th))
    sheet.save(path)


# ---------------------------------------------------------------------------------------------

def find_char(manifest, char_id):
    for char in manifest["characters"]:
        if char["id"] == char_id:
            return char
    ids = ", ".join(c["id"] for c in manifest["characters"])
    sys.exit(f"No existe el personaje «{char_id}». Los que hay: {ids}.")


def state_folder(char, state):
    return f"{char['folder']}/{state}"


def clear_dir(path):
    if os.path.isdir(path):
        shutil.rmtree(path)


def list_states(manifest):
    for char in manifest["characters"]:
        own = char.get("states", {})
        parts = []
        for state in STATES:
            spec = own.get(state)
            if spec is None:
                continue
            ms = f", {spec['ms']} ms/frame" if "ms" in spec else ""
            parts.append(f"{state} ({len(spec['sequence']) if 'sequence' in spec else spec['frames']} frames{' reutilizados' if 'sequence' in spec else ' propios'}{ms})")
        print(f"{char['id']:12} {', '.join(parts) if parts else 'solo la animación base'}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("character", nargs="?", help="id del personaje (el de assets/characters.json)")
    ap.add_argument("state", nargs="?", choices=STATES, help="estado al que pertenece la animación")
    ap.add_argument("sheet", nargs="?", help="imagen con los frames en cuadrícula")
    ap.add_argument("--cols", type=int, help="columnas de la cuadrícula")
    ap.add_argument("--rows", type=int, default=1, help="filas de la cuadrícula (por defecto 1)")
    ap.add_argument("--frames", type=int, help="cuántas celdas usar (por defecto, todas)")
    ap.add_argument("--split", choices=["auto", "grid"], default="auto", help="auto (por defecto): detecta cada personaje aunque estén pegados o desiguales; grid: celdas iguales")
    ap.add_argument("--bg", default="auto", help="fondo: auto (por defecto), none o un color #rrggbb")
    ap.add_argument("--tol", type=int, default=40, help="tolerancia del color de fondo, 0-255 (por defecto 40)")
    ap.add_argument("--trim", type=int, default=1, help="píxeles que se recortan del borde al quitar el fondo (por defecto 1; sube a 2-3 si queda un halo)")
    ap.add_argument("--ms", type=int, help="milisegundos por frame de esta animación (si no, sale de la duración típica del estado). Sin hoja, solo cambia el ritmo")
    ap.add_argument("--scale", type=float, default=1.0, help="multiplica el tamaño del personaje (1.1 = un 10 por ciento más grande; el máximo es lo que cabe en el lienzo)")
    ap.add_argument("--resample", choices=["lanczos", "nearest"], default="lanczos", help="al reescalar; nearest conserva el pixel art duro")
    ap.add_argument("--preview", help="guarda una vista de los frames resultantes en esta imagen")
    ap.add_argument("--remove", action="store_true", help="quita la animación propia de ese estado")
    ap.add_argument("--list", action="store_true", help="muestra los estados que tiene cada personaje")
    ap.add_argument("--assets", default=os.path.join(ROOT, "assets"), help="carpeta assets (por defecto, la del proyecto)")
    ap.add_argument("--dry-run", action="store_true", help="comprueba todo sin escribir nada")
    args = ap.parse_args()

    manifest_path = os.path.join(args.assets, "characters.json")
    manifest = read_manifest(manifest_path)
    if args.list:
        return list_states(manifest)
    if not args.character or not args.state:
        ap.error("indica el personaje y el estado (o usa --list)")
    char = find_char(manifest, args.character)
    folder = state_folder(char, args.state)
    out_dir = os.path.join(args.assets, *folder.split("/"))

    if args.remove:
        if args.state not in char.get("states", {}):
            sys.exit(f"{char['id']} no tiene una animación propia de «{args.state}».")
        if not args.dry_run:
            del char["states"][args.state]
            write_manifest(manifest_path, manifest)
            clear_dir(out_dir)
        print(f"Quitada la animación de «{args.state}» de {char['id']}.")
        return

    if args.ms is not None and not 20 <= args.ms <= 3000:
        ap.error("--ms debe estar entre 20 y 3000")
    if not args.sheet and args.ms is not None:  # solo se cambia el ritmo de una animación que ya existe
        spec = char.get("states", {}).get(args.state)
        if spec is None:
            sys.exit(f"{char['id']} no tiene una animación propia de «{args.state}».")
        if not args.dry_run:
            spec["ms"] = args.ms
            write_manifest(manifest_path, manifest)
        print(f"{char['id']}/{args.state}: ahora {args.ms} ms por frame.")
        return
    if not args.sheet or not args.cols:
        ap.error("faltan la hoja y --cols")
    if args.cols < 1 or args.rows < 1:
        ap.error("--cols y --rows deben ser 1 o más")
    frames_wanted = args.frames or args.cols * args.rows
    sheet = key_out(Image.open(args.sheet), args.bg, args.tol, args.trim)
    cells = extract_frames(sheet, args.cols, args.rows, frames_wanted, args.split)
    ref_path = os.path.join(args.assets, *char["folder"].split("/"), char["pattern"].replace("{nn}", f"{char['preview_frame']:02d}").replace("{n}", str(char["preview_frame"])))
    canvas, ref_box = reference_box(ref_path)
    frames = fit_frames(cells, canvas, ref_box, Image.LANCZOS if args.resample == "lanczos" else Image.NEAREST, args.scale)

    if args.preview:
        contact_sheet(frames, args.preview)
    if args.dry_run:
        print(f"Todo bien: {len(frames)} frames de {canvas[0]}x{canvas[1]} para {char['id']}/{args.state} (no se escribió nada).")
        return
    clear_dir(out_dir)
    os.makedirs(out_dir)
    pattern = f"{args.state}_{{nn}}.png"
    for i, frame in enumerate(frames, 1):
        frame.save(os.path.join(out_dir, pattern.replace("{nn}", f"{i:02d}")))
    ms = args.ms or max(40, round(CYCLE_MS[args.state] / len(frames)))
    char.setdefault("states", {})[args.state] = {"folder": folder, "pattern": pattern, "frames": len(frames), "ms": ms}
    write_manifest(manifest_path, manifest)
    print(f"{char['id']}/{args.state}: {len(frames)} frames de {canvas[0]}x{canvas[1]} en assets/{folder}/, a {ms} ms por frame")
    print(f"Si va muy rápido o lento: python tools/import_sprites.py {char['id']} {args.state} --ms <más = más lento>")
    print("Recompila la app (cargo build) para que se copien a la interfaz.")


if __name__ == "__main__":
    main()
