# Prompts para las animaciones de Peter Panda

Un prompt por estado, listos para pegar en tu IA de imágenes. En cada uno **adjunta la imagen de referencia**
`assets/peter-panda/sprite_01.png`. Después de generar cada hoja, impórtala con el comando que hay debajo del prompt
(más contexto y opciones en [animaciones.md](animaciones.md)).

Recomendación: empieza por **walking**, que es el que más se nota, y revisa con `--preview` antes de seguir con el resto.

## Bloque común

Todos los prompts empiezan igual. Está repetido en cada uno para que puedas copiarlos sueltos.

- Personaje: panda bebé de pixel art, cuerpo de cuatro patas, pelaje blanco con orejas, ojos, patas y cola negros, y una
  **bufanda roja** al cuello. Vista lateral mirando a la derecha.
- Formato: una sola fila de celdas cuadradas, personaje centrado y apoyado en el mismo suelo, fondo verde plano `#00ff00`.

---

## 1. walking — 6 frames

```text
Usa la imagen adjunta como referencia exacta del personaje: el panda bebé de pixel art con bufanda roja. Misma especie,
mismos colores, mismas proporciones, mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «caminar» de este personaje: un ciclo de caminar a cuatro patas, tranquilo y
simpático, con un ligero balanceo del cuerpo y de la bufanda. Los frames son: contacto, bajada, paso, contacto con las
patas contrarias, bajada, paso. El último frame enlaza con el primero para que sea un bucle.

Reglas:
- Exactamente 6 frames en UNA sola fila, de izquierda a derecha, en orden de animación.
- Cada frame ocupa una celda cuadrada del mismo tamaño. El personaje queda centrado en su celda y SIEMPRE apoyado en la, dale una separación considerable entre cada frame para que se pueda recortar bien después
  misma línea del suelo.
- Misma escala y mismo tamaño del personaje en todos los frames; solo cambia la pose.
- Vista lateral, mirando a la derecha, igual que la imagen de referencia.
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes.
```

```bash
python tools/import_sprites.py peter_panda walking hoja_walking.png --cols 6 --bg "#00ff00" --preview vista.png
```

## 2. idle — 4 frames

```text
Usa la imagen adjunta como referencia exacta del personaje: el panda bebé de pixel art con bufanda roja. Misma especie,
mismos colores, mismas proporciones, mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «reposo» de este personaje: está de pie, quieto, respirando suavemente (el
pecho sube y baja un poco), con un parpadeo en el tercer frame y un leve movimiento de la cola. El último frame enlaza
con el primero.

Reglas:
- Exactamente 4 frames en UNA sola fila, de izquierda a derecha, en orden de animación.
- Cada frame ocupa una celda cuadrada del mismo tamaño. El personaje queda centrado en su celda y SIEMPRE apoyado en la
  misma línea del suelo.
- Misma escala y mismo tamaño del personaje en todos los frames; solo cambia la pose.
- Vista lateral, mirando a la derecha, igual que la imagen de referencia.
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes.
```

```bash
python tools/import_sprites.py peter_panda idle hoja_idle.png --cols 4 --bg "#00ff00" --preview vista.png
```

## 3. sleeping — 6 frames

```text
Usa la imagen adjunta como referencia exacta del personaje: el panda bebé de pixel art con bufanda roja. Misma especie,
mismos colores, mismas proporciones, mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «dormir» de este personaje: está acurrucado en el suelo, hecho un ovillo, con
la cabeza apoyada sobre las patas delanteras y los ojos cerrados con una sonrisa tranquila. El cuerpo sube y baja muy
despacio al respirar (inspira, mantiene, espira, mantiene). La bufanda roja se ve claramente. El último frame enlaza con
el primero.

Reglas:
- Exactamente 6 frames en UNA sola fila, de izquierda a derecha, en orden de animación.
- Cada frame ocupa una celda cuadrada del mismo tamaño. El personaje queda centrado en su celda y SIEMPRE apoyado en la
  misma línea del suelo.
- Misma escala en todos los frames; solo cambia la respiración.
- Vista lateral, mirando a la derecha, igual que la imagen de referencia.
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes.
```

```bash
python tools/import_sprites.py peter_panda sleeping hoja_sleeping.png --cols 4 --bg "#00ff00" --preview vista.png
```

## 4. happy — 4 frames

```text
Usa la imagen adjunta como referencia exacta del personaje: el panda bebé de pixel art con bufanda roja. Misma especie,
mismos colores, mismas proporciones, mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «contento» de este personaje: sonríe con ojos brillantes y felices, rebota
suavemente sobre las cuatro patas (se agacha un poco, se estira, vuelve) y la bufanda ondea. Sin saltos grandes. El
último frame enlaza con el primero.

Reglas:
- Exactamente 4 frames en UNA sola fila, de izquierda a derecha, en orden de animación.
- Cada frame ocupa una celda cuadrada del mismo tamaño. El personaje queda centrado en su celda y apoyado en la misma
  línea del suelo (en el punto más alto del rebote las patas pueden separarse apenas).
- Misma escala y mismo tamaño del personaje en todos los frames; solo cambia la pose.
- Vista lateral, mirando a la derecha, igual que la imagen de referencia.
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes.
```

```bash
python tools/import_sprites.py peter_panda happy hoja_happy.png --cols 4 --bg "#00ff00" --preview vista.png
```

## 5. success — 6 frames

```text
Usa la imagen adjunta como referencia exacta del personaje: el panda bebé de pixel art con bufanda roja. Misma especie,
mismos colores, mismas proporciones, mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «celebrar un éxito» de este personaje: se agacha para tomar impulso, salta
hacia arriba con una gran sonrisa y los ojos cerrados de alegría, está en el aire con la bufanda al viento, baja,
aterriza aplastándose un poco y rebota una vez. El último frame vuelve a la pose de reposo.

Reglas:
- Exactamente 6 frames en UNA sola fila, de izquierda a derecha, en orden de animación.
- Cada frame ocupa una celda cuadrada del mismo tamaño, con espacio arriba para el salto. Fuera del salto, el personaje
  se apoya siempre en la misma línea del suelo.
- Misma escala y mismo tamaño del personaje en todos los frames; solo cambia la pose.
- Vista frontal, igual que la imagen de referencia.
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes.
```

```bash
python tools/import_sprites.py peter_panda success hoja_success.png --cols 6 --bg "#00ff00" --preview vista.png
```

## 6. thinking — 6 frames

```text
Usa la imagen adjunta como referencia exacta del personaje: el panda bebé de pixel art con bufanda roja. Misma especie,
mismos colores, mismas proporciones, mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «pensar» de este personaje: está de pie, ladea la cabeza poco a poco, mira
hacia arriba y hacia un lado con una expresión curiosa, se rasca la barbilla con una pata delantera y vuelve a mirar al
frente. El último frame enlaza con el primero.

Reglas:
- Exactamente 6 frames en UNA sola fila, de izquierda a derecha, en orden de animación.
- Cada frame ocupa una celda cuadrada del mismo tamaño. El personaje queda centrado en su celda y SIEMPRE apoyado en la
  misma línea del suelo.
- Misma escala y mismo tamaño del personaje en todos los frames; solo cambia la pose.
- Vista lateral, mirando a la derecha, igual que la imagen de referencia.
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes.
```

```bash
python tools/import_sprites.py peter_panda thinking hoja_thinking.png --cols 6 --bg "#00ff00" --preview vista.png
```

## 7. working — 4 frames

```text
Usa la imagen adjunta como referencia exacta del personaje: el panda bebé de pixel art con bufanda roja. Misma especie,
mismos colores, mismas proporciones, mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «trabajar» de este personaje: está concentrado, con el ceño un poco fruncido y
la vista al frente, y mueve las patas delanteras alternadamente como si tecleara en un teclado invisible, con un pequeño
rebote del cuerpo en cada golpe. El último frame enlaza con el primero.

Reglas:
- Exactamente 4 frames en UNA sola fila, de izquierda a derecha, en orden de animación.
- Cada frame ocupa una celda cuadrada del mismo tamaño. El personaje queda centrado en su celda y SIEMPRE apoyado en la
  misma línea del suelo.
- Misma escala y mismo tamaño del personaje en todos los frames; solo cambia la pose.
- Vista lateral, mirando a la derecha, igual que la imagen de referencia.
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes.
```

```bash
python tools/import_sprites.py peter_panda working hoja_working.png --cols 4 --bg "#00ff00" --preview vista.png
```

## 8. surprised — 4 frames

```text
Usa la imagen adjunta como referencia exacta del personaje: el panda bebé de pixel art con bufanda roja. Misma especie,
mismos colores, mismas proporciones, mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «sorpresa» de este personaje: de golpe abre mucho los ojos y la boca, da un
pequeño salto hacia atrás con las orejas y la bufanda levantadas, se queda un instante tieso y vuelve a bajar. El último
frame vuelve casi a la pose de reposo.

Reglas:
- Exactamente 4 frames en UNA sola fila, de izquierda a derecha, en orden de animación.
- Cada frame ocupa una celda cuadrada del mismo tamaño, con un poco de espacio arriba. Fuera del salto, el personaje se
  apoya siempre en la misma línea del suelo.
- Misma escala y mismo tamaño del personaje en todos los frames; solo cambia la pose.
- Vista lateral, mirando a la derecha, igual que la imagen de referencia.
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes.
```

```bash
python tools/import_sprites.py peter_panda surprised hoja_surprised.png --cols 4 --bg "#00ff00" --preview vista.png
```

## 9. interacting — 4 frames

```text
Usa la imagen adjunta como referencia exacta del personaje: el panda bebé de pixel art con bufanda roja. Misma especie,
mismos colores, mismas proporciones, mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «saludar» de este personaje: gira la cabeza hacia quien lo toca (hacia la
cámara), sonríe, levanta una pata delantera para saludar y la mueve de un lado a otro. El último frame enlaza con el
primero.

Reglas:
- Exactamente 4 frames en UNA sola fila, de izquierda a derecha, en orden de animación.
- Cada frame ocupa una celda cuadrada del mismo tamaño. El personaje queda centrado en su celda y SIEMPRE apoyado en la
  misma línea del suelo.
- Misma escala y mismo tamaño del personaje en todos los frames; solo cambia la pose.
- Cuerpo en vista lateral mirando a la derecha; solo la cabeza se gira hacia el frente.
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes.
```

```bash
python tools/import_sprites.py peter_panda interacting hoja_interacting.png --cols 4 --bg "#00ff00" --preview vista.png
```

## 10. alert — 4 frames

```text
Usa la imagen adjunta como referencia exacta del personaje: el panda bebé de pixel art con bufanda roja. Misma especie,
mismos colores, mismas proporciones, mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «alerta» de este personaje: se pone firme y erguido sobre las patas traseras,
con las orejas muy tiesas y los ojos grandes y atentos, y mira de un lado a otro como avisando de algo; las patas
delanteras hacen un gesto de «¡atención!». El último frame enlaza con el primero.

Reglas:
- Exactamente 4 frames en UNA sola fila, de izquierda a derecha, en orden de animación.
- Cada frame ocupa una celda cuadrada del mismo tamaño. El personaje queda centrado en su celda y SIEMPRE apoyado en la
  misma línea del suelo.
- Misma escala y mismo tamaño del personaje en todos los frames; solo cambia la pose.
- Vista lateral, mirando a la derecha, igual que la imagen de referencia.
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes.
```

```bash
python tools/import_sprites.py peter_panda alert hoja_alert.png --cols 4 --bg "#00ff00" --preview vista.png
```

## 11. listening — 6 frames

```text
Usa la imagen adjunta como referencia exacta del personaje: el panda bebé de pixel art con bufanda roja. Misma especie,
mismos colores, mismas proporciones, mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «escuchar música» de este personaje: tiene los ojos cerrados con una sonrisa
relajada y cabecea al ritmo, inclinando el cuerpo de un lado a otro con un pequeño rebote. NO dibujes auriculares ni
notas musicales: se añaden aparte. El último frame enlaza con el primero.

Reglas:
- Exactamente 6 frames en UNA sola fila, de izquierda a derecha, en orden de animación.
- Cada frame ocupa una celda cuadrada del mismo tamaño. El personaje queda centrado en su celda y SIEMPRE apoyado en la
  misma línea del suelo.
- Misma escala y mismo tamaño del personaje en todos los frames; solo cambia la pose.
- Vista lateral, mirando a la derecha, igual que la imagen de referencia.
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes.
```

> La app ya pone unos auriculares encima del personaje cuando suena música (los de `assets/headphones/peter_panda.png`),
> por eso este prompt pide que **no** los dibuje: si saliesen en el dibujo, se verían dobles.

```bash
python tools/import_sprites.py peter_panda listening hoja_listening.png --cols 6 --bg "#00ff00" --preview vista.png
```
