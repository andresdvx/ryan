#!/usr/bin/env python3
"""Escribe docs/prompts-<personaje>.md: un prompt por estado, listo para pegar en una IA de imágenes, con el comando
para importar cada hoja. Pensado para personajes de pie sobre dos patas (de frente, como sus sprites de reposo).

Uso (desde la raíz del proyecto):

    python tools/make_prompts.py pingu bandit mochi ellie drako          # los que ya trae la app
    python tools/make_prompts.py nuevo --desc "zorro azul con bufanda amarilla" --flutter "la bufanda" --tail "la cola"

Para un personaje nuevo, --desc es lo único obligatorio: cómo es, en una frase (especie, colores, rasgos y accesorios).
"""
import argparse
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# Cómo es cada personaje que trae la app. «flutter» es lo que ondea al moverse; «tail», su cola (o None);
# «alert» es lo que hace al ponerse en alerta; «walk» es cómo anda.
KNOWN = {
    "pingu": {
        "desc": "pingüino bebé de pixel art, cuerpo redondo negro, barriga y cara blancas, pico y patas naranjas, mejillas rosas y un mechón de plumas en la cabeza. No lleva bufanda",
        "flutter": "el mechón de plumas de la cabeza", "tail": None,
        "alert": "el mechón de plumas se eriza y las aletas se separan del cuerpo",
        "walk": "anda como un pingüino, contoneándose de un lado a otro, con las aletas algo separadas y pasitos cortos sobre los pies naranjas",
    },
    "bandit": {
        "desc": "mapache bebé de pixel art, pelaje gris, antifaz oscuro alrededor de los ojos, orejas redondeadas, barriga clara, cola con anillos oscuros y una bufanda de rayas de colores (rojo, amarillo, verde, azul y morado)",
        "flutter": "la bufanda", "tail": "la cola de anillos",
        "alert": "las orejas se ponen tiesas y los ojos se abren atentos",
        "walk": "camina erguido sobre las dos patas traseras, con pasos tranquilos y simpáticos, balanceando los brazos y la cola de anillos",
    },
    "mochi": {
        "desc": "panda rojo bebé de pixel art, pelaje naranja, cara y orejas con marcas blancas, patas y barriga oscuras, cola grande con anillos y una bufanda azul",
        "flutter": "la bufanda", "tail": "la cola de anillos",
        "alert": "las orejas se ponen tiesas y los ojos se abren atentos",
        "walk": "camina erguido sobre las dos patas traseras, con pasos tranquilos y simpáticos, balanceando los brazos y la cola de anillos",
    },
    "ellie": {
        "desc": "nutria bebé de pixel art, pelaje marrón, barriga y hocico beige, orejitas redondas, mejillas rosadas, cola gruesa y un pañuelo verde con lazo al cuello",
        "flutter": "el pañuelo", "tail": "la cola gruesa",
        "alert": "las orejitas se ponen tiesas y los ojos se abren atentos",
        "walk": "camina erguida sobre las dos patas traseras, con pasos suaves y juguetones, balanceando los brazos y la cola",
    },
    "drako": {
        "desc": "dragón bebé de pixel art, cuerpo verde, barriga amarilla, ojos muy grandes, cresta pequeña en la cabeza, garras blancas, cola con púas y una bufanda naranja",
        "flutter": "la bufanda", "tail": "la cola con púas",
        "alert": "la cresta se eriza y los ojos se abren atentos",
        "walk": "camina erguido sobre las dos patas traseras, con pasos firmes y graciosos, balanceando los brazos y la cola con púas",
    },
}

# estado -> (frames, vista, descripción). {name}, {flutter} y {tail_move} se rellenan por personaje.
STATES = [
    ("walking", 6, "side", "{walk}. Ciclo de caminar completo de 6 frames: contacto, bajada, paso, contacto con el otro pie, bajada, paso. {flutter_cap} ondea ligeramente al andar. El último frame enlaza con el primero"),
    ("idle", 4, "front", "de pie, quieto, respirando suavemente (el pecho sube y baja un poco) y con un parpadeo en el tercer frame{tail_move}. El último frame enlaza con el primero"),
    ("sleeping", 4, "front", "acurrucado en el suelo, hecho un ovillo con la cabeza apoyada, los ojos cerrados y una sonrisa tranquila. El cuerpo sube y baja muy despacio al respirar (inspira, mantiene, espira, mantiene). {flutter_cap} se ve claramente. El último frame enlaza con el primero"),
    ("happy", 6, "front", "sonríe con ojos brillantes y felices, rebota suavemente sobre los pies (se agacha un poco, se estira, vuelve) y levanta un poco los brazos; {flutter} ondea. Sin saltos grandes. El último frame enlaza con el primero"),
    ("success", 6, "front", "celebra un éxito: se agacha para tomar impulso, salta hacia arriba con una gran sonrisa y los brazos en alto, está en el aire con {flutter} al viento, baja, aterriza aplastándose un poco y rebota una vez. El último frame vuelve a la pose de reposo"),
    ("thinking", 6, "front", "está de pie, ladea la cabeza poco a poco, mira hacia arriba y hacia un lado con cara de curiosidad, se lleva una mano a la barbilla y vuelve a mirar al frente. El último frame enlaza con el primero"),
    ("working", 4, "front", "está concentrado, con el ceño un poco fruncido, y mueve los brazos alternadamente como si tecleara en un teclado invisible, con un pequeño rebote del cuerpo en cada golpe. El último frame enlaza con el primero"),
    ("surprised", 4, "front", "de golpe abre mucho los ojos y la boca, da un pequeño salto hacia atrás con los brazos levantados y {flutter} al aire, se queda un instante tieso y vuelve a bajar. El último frame vuelve casi a la pose de reposo"),
    ("interacting", 4, "front", "mira hacia la cámara, sonríe, levanta un brazo para saludar y lo mueve de un lado a otro. El último frame enlaza con el primero"),
    ("alert", 4, "front", "se pone firme y erguido con los ojos grandes y atentos: {alert}; mira de un lado a otro como avisando de algo y los brazos hacen un gesto de «¡atención!». El último frame enlaza con el primero"),
    ("listening", 6, "front", "tiene los ojos cerrados con una sonrisa relajada y cabecea al ritmo, inclinando el cuerpo de un lado a otro con un pequeño rebote. NO dibujes auriculares ni notas musicales: se añaden aparte. El último frame enlaza con el primero"),
]

VIEWS = {
    "front": "Vista frontal, igual que la imagen de referencia.",
    "side": "Vista de tres cuartos mirando hacia la DERECHA: el cuerpo de lado y la cara visible. La app voltea el dibujo al caminar hacia la izquierda, así que siempre hacia la derecha.",
}

PROMPT = """Usa la imagen adjunta como referencia exacta del personaje: {desc}. Misma especie, mismos colores, mismas
proporciones, mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «{label}» de este personaje: {text}.

Reglas:
- Exactamente {n} frames en UNA sola fila, de izquierda a derecha, en orden de animación.
- Cada frame ocupa una celda cuadrada del mismo tamaño. El personaje queda centrado en su celda y SIEMPRE apoyado en la
  misma línea del suelo (salvo en un salto).
- Misma escala y mismo tamaño del personaje en todos los frames; solo cambia la pose.
- {view}
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Sin líneas sueltas, marcas ni numeración entre los frames.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes."""

LABELS = {"walking": "caminar", "idle": "reposo", "sleeping": "dormir", "happy": "contento", "success": "celebrar un éxito",
          "thinking": "pensar", "working": "trabajar", "surprised": "sorpresa", "interacting": "saludar", "alert": "alerta",
          "listening": "escuchar música"}


def capital(text):
    return text[:1].upper() + text[1:]


def build(char_id, name, info):
    flutter, tail = info["flutter"], info.get("tail")
    fill = dict(flutter=flutter, flutter_cap=capital(flutter), walk=info.get("walk") or f"camina erguido sobre las dos patas traseras, con pasos tranquilos y simpáticos y {flutter} ondeando",
                alert=info.get("alert") or "se pone tieso y atento", tail_move=f" y un leve movimiento de {tail}" if tail else "")
    out = [f"# Prompts para las animaciones de {name}\n",
           f"Un prompt por estado, listos para pegar en tu IA de imágenes. En cada uno **adjunta la imagen de referencia** "
           f"`assets/{char_id}/sprite_01.png`. Después de generar cada hoja, impórtala con el comando que hay debajo del prompt "
           f"(más contexto y opciones en [animaciones.md](animaciones.md)).\n",
           "Orden recomendado: **walking**, **idle**, **sleeping** y **happy**, que son los que más se ven. Revisa siempre con "
           "`--preview` antes de seguir con el siguiente. Si algo va muy rápido o lento, cambia el ritmo con "
           f"`python tools/import_sprites.py {char_id} <estado> --ms <milisegundos por frame>`.\n",
           "> **Ojo con la vista:** los sprites de este personaje son de frente. El paseo (`walking`) es el único que se pide "
           "de tres cuartos mirando a la derecha, porque se desplaza por el escritorio; el resto mantiene la vista frontal.\n",
           "---\n"]
    for i, (state, n, view, text) in enumerate(STATES, 1):
        body = PROMPT.format(desc=info["desc"], label=LABELS[state], n=n, view=VIEWS[view], text=text.format(**fill))
        out.append(f"## {i}. {state} — {n} frames\n")
        out.append(f"```text\n{body}\n```\n")
        out.append(f"```bash\npython tools/import_sprites.py {char_id} {state} hoja_{state}.png --cols {n} --bg \"#00ff00\" --preview vista.png\n```\n")
        if state == "listening":
            out.append("> La app ya pone unos auriculares encima del personaje cuando suena música, por eso este prompt pide que no los dibuje.\n")
    return "\n".join(out)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("characters", nargs="+", help="ids de personaje (los de assets/characters.json)")
    ap.add_argument("--desc", help="cómo es el personaje (obligatorio si no es uno de los que trae la app)")
    ap.add_argument("--flutter", help="qué ondea al moverse, por ejemplo «la bufanda»")
    ap.add_argument("--tail", help="su cola, por ejemplo «la cola de anillos» (omítelo si no tiene)")
    ap.add_argument("--walk", help="cómo anda, en una frase (opcional)")
    args = ap.parse_args()
    manifest = json.load(open(os.path.join(ROOT, "assets", "characters.json"), encoding="utf-8"))
    names = {c["id"]: c["name"] for c in manifest["characters"]}
    for char_id in args.characters:
        if char_id not in names:
            sys.exit(f"No existe el personaje «{char_id}» en assets/characters.json. Los que hay: {', '.join(names)}.")
        info = dict(KNOWN.get(char_id, {}))
        if args.desc:
            info.update(desc=args.desc, flutter=args.flutter or info.get("flutter") or "su accesorio", tail=args.tail or info.get("tail"))
            if args.walk:
                info["walk"] = args.walk
        if "desc" not in info:
            sys.exit(f"«{char_id}» no está en la tabla: descríbelo con --desc \"...\" (y opcionalmente --flutter, --tail, --walk).")
        path = os.path.join(ROOT, "docs", f"prompts-{char_id.replace('_', '-')}.md")
        with open(path, "wb") as out:
            out.write(build(char_id, names[char_id], info).replace("\n", "\r\n").encode("utf-8"))
        print("escrito", os.path.relpath(path, ROOT))


if __name__ == "__main__":
    main()
