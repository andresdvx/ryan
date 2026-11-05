# Prompts para las animaciones de Mochi

Un prompt por estado, listos para pegar en tu IA de imágenes. En cada uno **adjunta la imagen de referencia** `assets/mochi/sprite_01.png`. Después de generar cada hoja, impórtala con el comando que hay debajo del prompt (más contexto y opciones en [animaciones.md](animaciones.md)).

Orden recomendado: **walking**, **idle**, **sleeping** y **happy**, que son los que más se ven. Revisa siempre con `--preview` antes de seguir con el siguiente. Si algo va muy rápido o lento, cambia el ritmo con `python tools/import_sprites.py mochi <estado> --ms <milisegundos por frame>`.

> **Ojo con la vista:** los sprites de este personaje son de frente. El paseo (`walking`) es el único que se pide de tres cuartos mirando a la derecha, porque se desplaza por el escritorio; el resto mantiene la vista frontal.

---

## 1. walking — 6 frames

```text
Usa la imagen adjunta como referencia exacta del personaje: panda rojo bebé de pixel art, pelaje naranja, cara y orejas con marcas blancas, patas y barriga oscuras, cola grande con anillos y una bufanda azul. Misma especie, mismos colores, mismas
proporciones, mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «caminar» de este personaje: camina erguido sobre las dos patas traseras, con pasos tranquilos y simpáticos, balanceando los brazos y la cola de anillos. Ciclo de caminar completo de 6 frames: contacto, bajada, paso, contacto con el otro pie, bajada, paso. La bufanda ondea ligeramente al andar. El último frame enlaza con el primero.

Reglas:
- Exactamente 6 frames en UNA sola fila, de izquierda a derecha, en orden de animación.
- Cada frame ocupa una celda cuadrada del mismo tamaño. El personaje queda centrado en su celda y SIEMPRE apoyado en la
  misma línea del suelo (salvo en un salto).
- Misma escala y mismo tamaño del personaje en todos los frames; solo cambia la pose.
- Vista de tres cuartos mirando hacia la DERECHA: el cuerpo de lado y la cara visible. La app voltea el dibujo al caminar hacia la izquierda, así que siempre hacia la derecha.
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Sin líneas sueltas, marcas ni numeración entre los frames.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes.
```

```bash
python tools/import_sprites.py mochi walking hoja_walking.png --cols 6 --bg "#00ff00" --preview vista.png
```

## 2. idle — 4 frames

```text
Usa la imagen adjunta como referencia exacta del personaje: panda rojo bebé de pixel art, pelaje naranja, cara y orejas con marcas blancas, patas y barriga oscuras, cola grande con anillos y una bufanda azul. Misma especie, mismos colores, mismas
proporciones, mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «reposo» de este personaje: de pie, quieto, respirando suavemente (el pecho sube y baja un poco) y con un parpadeo en el tercer frame y un leve movimiento de la cola de anillos. El último frame enlaza con el primero.

Reglas:
- Exactamente 4 frames en UNA sola fila, de izquierda a derecha, en orden de animación.
- Cada frame ocupa una celda cuadrada del mismo tamaño. El personaje queda centrado en su celda y SIEMPRE apoyado en la
  misma línea del suelo (salvo en un salto).
- Misma escala y mismo tamaño del personaje en todos los frames; solo cambia la pose.
- Vista frontal, igual que la imagen de referencia.
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Sin líneas sueltas, marcas ni numeración entre los frames.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes.
```

```bash
python tools/import_sprites.py mochi idle hoja_idle.png --cols 4 --bg "#00ff00" --preview vista.png
```

## 3. sleeping — 4 frames

```text
Usa la imagen adjunta como referencia exacta del personaje: panda rojo bebé de pixel art, pelaje naranja, cara y orejas con marcas blancas, patas y barriga oscuras, cola grande con anillos y una bufanda azul. Misma especie, mismos colores, mismas
proporciones, mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «dormir» de este personaje: acurrucado en el suelo, hecho un ovillo con la cabeza apoyada, los ojos cerrados y una sonrisa tranquila. El cuerpo sube y baja muy despacio al respirar (inspira, mantiene, espira, mantiene). La bufanda se ve claramente. El último frame enlaza con el primero.

Reglas:
- Exactamente 4 frames en UNA sola fila, de izquierda a derecha, en orden de animación.
- Cada frame ocupa una celda cuadrada del mismo tamaño. El personaje queda centrado en su celda y SIEMPRE apoyado en la
  misma línea del suelo (salvo en un salto).
- Misma escala y mismo tamaño del personaje en todos los frames; solo cambia la pose.
- Vista frontal, igual que la imagen de referencia.
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Sin líneas sueltas, marcas ni numeración entre los frames.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes.
```

```bash
python tools/import_sprites.py mochi sleeping hoja_sleeping.png --cols 4 --bg "#00ff00" --preview vista.png
```

## 4. happy — 6 frames

```text
Usa la imagen adjunta como referencia exacta del personaje: panda rojo bebé de pixel art, pelaje naranja, cara y orejas con marcas blancas, patas y barriga oscuras, cola grande con anillos y una bufanda azul. Misma especie, mismos colores, mismas
proporciones, mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «contento» de este personaje: sonríe con ojos brillantes y felices, rebota suavemente sobre los pies (se agacha un poco, se estira, vuelve) y levanta un poco los brazos; la bufanda ondea. Sin saltos grandes. El último frame enlaza con el primero.

Reglas:
- Exactamente 6 frames en UNA sola fila, de izquierda a derecha, en orden de animación.
- Cada frame ocupa una celda cuadrada del mismo tamaño. El personaje queda centrado en su celda y SIEMPRE apoyado en la
  misma línea del suelo (salvo en un salto).
- Misma escala y mismo tamaño del personaje en todos los frames; solo cambia la pose.
- Vista frontal, igual que la imagen de referencia.
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Sin líneas sueltas, marcas ni numeración entre los frames.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes.
```

```bash
python tools/import_sprites.py mochi happy hoja_happy.png --cols 6 --bg "#00ff00" --preview vista.png
```

## 5. success — 6 frames

```text
Usa la imagen adjunta como referencia exacta del personaje: panda rojo bebé de pixel art, pelaje naranja, cara y orejas con marcas blancas, patas y barriga oscuras, cola grande con anillos y una bufanda azul. Misma especie, mismos colores, mismas
proporciones, mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «celebrar un éxito» de este personaje: celebra un éxito: se agacha para tomar impulso, salta hacia arriba con una gran sonrisa y los brazos en alto, está en el aire con la bufanda al viento, baja, aterriza aplastándose un poco y rebota una vez. El último frame vuelve a la pose de reposo.

Reglas:
- Exactamente 6 frames en UNA sola fila, de izquierda a derecha, en orden de animación.
- Cada frame ocupa una celda cuadrada del mismo tamaño. El personaje queda centrado en su celda y SIEMPRE apoyado en la
  misma línea del suelo (salvo en un salto).
- Misma escala y mismo tamaño del personaje en todos los frames; solo cambia la pose.
- Vista frontal, igual que la imagen de referencia.
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Sin líneas sueltas, marcas ni numeración entre los frames.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes.
```

```bash
python tools/import_sprites.py mochi success hoja_success.png --cols 6 --bg "#00ff00" --preview vista.png
```

## 6. thinking — 6 frames

```text
Usa la imagen adjunta como referencia exacta del personaje: panda rojo bebé de pixel art, pelaje naranja, cara y orejas con marcas blancas, patas y barriga oscuras, cola grande con anillos y una bufanda azul. Misma especie, mismos colores, mismas
proporciones, mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «pensar» de este personaje: está de pie, ladea la cabeza poco a poco, mira hacia arriba y hacia un lado con cara de curiosidad, se lleva una mano a la barbilla y vuelve a mirar al frente. El último frame enlaza con el primero.

Reglas:
- Exactamente 6 frames en UNA sola fila, de izquierda a derecha, en orden de animación.
- Cada frame ocupa una celda cuadrada del mismo tamaño. El personaje queda centrado en su celda y SIEMPRE apoyado en la
  misma línea del suelo (salvo en un salto).
- Misma escala y mismo tamaño del personaje en todos los frames; solo cambia la pose.
- Vista frontal, igual que la imagen de referencia.
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Sin líneas sueltas, marcas ni numeración entre los frames.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes.
```

```bash
python tools/import_sprites.py mochi thinking hoja_thinking.png --cols 6 --bg "#00ff00" --preview vista.png
```

## 7. working — 4 frames

```text
Usa la imagen adjunta como referencia exacta del personaje: panda rojo bebé de pixel art, pelaje naranja, cara y orejas con marcas blancas, patas y barriga oscuras, cola grande con anillos y una bufanda azul. Misma especie, mismos colores, mismas
proporciones, mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «trabajar» de este personaje: está concentrado, con el ceño un poco fruncido, y mueve los brazos alternadamente como si tecleara en un teclado invisible, con un pequeño rebote del cuerpo en cada golpe. El último frame enlaza con el primero.

Reglas:
- Exactamente 4 frames en UNA sola fila, de izquierda a derecha, en orden de animación.
- Cada frame ocupa una celda cuadrada del mismo tamaño. El personaje queda centrado en su celda y SIEMPRE apoyado en la
  misma línea del suelo (salvo en un salto).
- Misma escala y mismo tamaño del personaje en todos los frames; solo cambia la pose.
- Vista frontal, igual que la imagen de referencia.
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Sin líneas sueltas, marcas ni numeración entre los frames.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes.
```

```bash
python tools/import_sprites.py mochi working hoja_working.png --cols 4 --bg "#00ff00" --preview vista.png
```

## 8. surprised — 4 frames

```text
Usa la imagen adjunta como referencia exacta del personaje: panda rojo bebé de pixel art, pelaje naranja, cara y orejas con marcas blancas, patas y barriga oscuras, cola grande con anillos y una bufanda azul. Misma especie, mismos colores, mismas
proporciones, mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «sorpresa» de este personaje: de golpe abre mucho los ojos y la boca, da un pequeño salto hacia atrás con los brazos levantados y la bufanda al aire, se queda un instante tieso y vuelve a bajar. El último frame vuelve casi a la pose de reposo.

Reglas:
- Exactamente 4 frames en UNA sola fila, de izquierda a derecha, en orden de animación.
- Cada frame ocupa una celda cuadrada del mismo tamaño. El personaje queda centrado en su celda y SIEMPRE apoyado en la
  misma línea del suelo (salvo en un salto).
- Misma escala y mismo tamaño del personaje en todos los frames; solo cambia la pose.
- Vista frontal, igual que la imagen de referencia.
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Sin líneas sueltas, marcas ni numeración entre los frames.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes.
```

```bash
python tools/import_sprites.py mochi surprised hoja_surprised.png --cols 4 --bg "#00ff00" --preview vista.png
```

## 9. interacting — 4 frames

```text
Usa la imagen adjunta como referencia exacta del personaje: panda rojo bebé de pixel art, pelaje naranja, cara y orejas con marcas blancas, patas y barriga oscuras, cola grande con anillos y una bufanda azul. Misma especie, mismos colores, mismas
proporciones, mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «saludar» de este personaje: mira hacia la cámara, sonríe, levanta un brazo para saludar y lo mueve de un lado a otro. El último frame enlaza con el primero.

Reglas:
- Exactamente 4 frames en UNA sola fila, de izquierda a derecha, en orden de animación.
- Cada frame ocupa una celda cuadrada del mismo tamaño. El personaje queda centrado en su celda y SIEMPRE apoyado en la
  misma línea del suelo (salvo en un salto).
- Misma escala y mismo tamaño del personaje en todos los frames; solo cambia la pose.
- Vista frontal, igual que la imagen de referencia.
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Sin líneas sueltas, marcas ni numeración entre los frames.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes.
```

```bash
python tools/import_sprites.py mochi interacting hoja_interacting.png --cols 4 --bg "#00ff00" --preview vista.png
```

## 10. alert — 4 frames

```text
Usa la imagen adjunta como referencia exacta del personaje: panda rojo bebé de pixel art, pelaje naranja, cara y orejas con marcas blancas, patas y barriga oscuras, cola grande con anillos y una bufanda azul. Misma especie, mismos colores, mismas
proporciones, mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «alerta» de este personaje: se pone firme y erguido con los ojos grandes y atentos: las orejas se ponen tiesas y los ojos se abren atentos; mira de un lado a otro como avisando de algo y los brazos hacen un gesto de «¡atención!». El último frame enlaza con el primero.

Reglas:
- Exactamente 4 frames en UNA sola fila, de izquierda a derecha, en orden de animación.
- Cada frame ocupa una celda cuadrada del mismo tamaño. El personaje queda centrado en su celda y SIEMPRE apoyado en la
  misma línea del suelo (salvo en un salto).
- Misma escala y mismo tamaño del personaje en todos los frames; solo cambia la pose.
- Vista frontal, igual que la imagen de referencia.
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Sin líneas sueltas, marcas ni numeración entre los frames.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes.
```

```bash
python tools/import_sprites.py mochi alert hoja_alert.png --cols 4 --bg "#00ff00" --preview vista.png
```

## 11. listening — 6 frames

```text
Usa la imagen adjunta como referencia exacta del personaje: panda rojo bebé de pixel art, pelaje naranja, cara y orejas con marcas blancas, patas y barriga oscuras, cola grande con anillos y una bufanda azul. Misma especie, mismos colores, mismas
proporciones, mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «escuchar música» de este personaje: tiene los ojos cerrados con una sonrisa relajada y cabecea al ritmo, inclinando el cuerpo de un lado a otro con un pequeño rebote. NO dibujes auriculares ni notas musicales: se añaden aparte. El último frame enlaza con el primero.

Reglas:
- Exactamente 6 frames en UNA sola fila, de izquierda a derecha, en orden de animación.
- Cada frame ocupa una celda cuadrada del mismo tamaño. El personaje queda centrado en su celda y SIEMPRE apoyado en la
  misma línea del suelo (salvo en un salto).
- Misma escala y mismo tamaño del personaje en todos los frames; solo cambia la pose.
- Vista frontal, igual que la imagen de referencia.
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Sin líneas sueltas, marcas ni numeración entre los frames.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes.
```

```bash
python tools/import_sprites.py mochi listening hoja_listening.png --cols 6 --bg "#00ff00" --preview vista.png
```

> La app ya pone unos auriculares encima del personaje cuando suena música, por eso este prompt pide que no los dibuje.
