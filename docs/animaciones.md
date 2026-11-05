# Animaciones propias por estado

Cada amigo se anima según lo que hace: reposo, caminando, pensando, trabajando, éxito, contento, sorprendido, alerta,
durmiendo… Por defecto reutiliza los frames que ya tiene (cambiando caras y con movimiento por código). Si quieres que
**camine de verdad, duerma acurrucado o salte al celebrar**, puedes darle una animación dibujada para cada estado.

El flujo es siempre el mismo:

1. Generas **una hoja de sprites por estado** con una IA de imágenes (más abajo, el prompt).
2. La importas con `tools/import_sprites.py`: corta los frames, quita el fondo, los alinea y declara el estado.
3. Recompilas (`cargo build`) y listo.

## Qué frames pedir

| Estado | Cuándo se ve | Frames | Qué dibujar |
| --- | --- | --- | --- |
| `idle` | Reposo | 4–6 | Respirar, parpadear, mover un poco la cola o las orejas |
| `walking` | Pasea por el escritorio | 6–8 | Ciclo de paso completo (contacto, bajada, paso, subida), mirando a la **derecha** |
| `thinking` | Esperando a la IA | 4–6 | Mirar arriba, mano en la barbilla, ladear la cabeza |
| `working` | Ejecutando una acción | 4–6 | Concentrado, moviendo patas o manos como si tecleara |
| `success` | Acción terminada | 4–6 | Salto de alegría y aterrizaje |
| `happy` | Contento | 4–6 | Sonrisa grande, rebote suave |
| `surprised` | Algo inesperado | 3–4 | Ojos muy abiertos, pequeño salto hacia atrás |
| `interacting` | Le haces clic | 4 | Saludo o giro de cabeza hacia ti |
| `alert` | Aviso o recordatorio | 4 | Pone la oreja o se pone firme, ojos atentos |
| `sleeping` | Mucho rato sin actividad | 4 | Acurrucado, respirando despacio, con los ojos cerrados |
| `listening` | Suena música (en reposo) | 4–6 | Cabeceando al ritmo, ojos cerrados |

Con 4–8 frames por estado basta: la app los reproduce en bucle a la velocidad que corresponde.

## Prompt para la IA de imágenes

Pásale **la imagen de tu personaje** (por ejemplo `assets/peter-panda/sprite_01.png`) y este texto, cambiando lo que va
entre `[corchetes]`:

```text
Usa la imagen adjunta como referencia exacta del personaje: misma especie, mismos colores, mismas proporciones,
mismo estilo de pixel art y mismo grosor de contorno. No cambies su diseño.

Dibuja una HOJA DE SPRITES de la animación «[ESTADO]» de este personaje: [DESCRIPCIÓN DEL MOVIMIENTO].

Reglas:
- Exactamente [N] frames en UNA sola fila, de izquierda a derecha, en orden de animación, y que el último frame
  enlace con el primero (bucle).
- Cada frame ocupa una celda cuadrada del mismo tamaño. El personaje queda centrado en su celda y SIEMPRE apoyado
  en la misma línea del suelo (las patas no se hunden ni flotan, salvo en un salto).
- Misma escala y mismo tamaño del personaje en todos los frames; solo cambia la pose.
- Vista lateral, mirando a la derecha, igual que la imagen de referencia.
- Fondo de un único color plano #00ff00 (verde puro), sin degradados, sin sombras en el suelo y sin marcos ni texto.
- Pixel art nítido: sin desenfoque ni antialiasing en los bordes.
```

Ejemplos de la parte variable:

- **walking**, N = 6: «ciclo de caminar con cuatro patas: contacto, bajada, paso, subida, con un ligero balanceo del cuerpo»
- **sleeping**, N = 4: «acurrucado en el suelo con los ojos cerrados; el cuerpo sube y baja muy despacio al respirar»
- **success**, N = 5: «agacha el cuerpo, salta con los brazos arriba, está en el aire, aterriza, rebota una vez»
- **thinking**, N = 4: «mira hacia arriba y a un lado, con una pata en la barbilla; la cabeza se ladea poco a poco»

> Consejo: pide **un estado por hoja**. Las IA mantienen mucho mejor la coherencia con 4–8 frames que con una hoja
> gigante de todos los estados. Si un frame sale mal, regenera la hoja entera en vez de arreglarlo a mano.

## Importar la hoja

Necesitas Pillow una vez: `pip install pillow`.

```bash
python tools/import_sprites.py peter_panda walking hoja_caminar.png --cols 6 --bg "#00ff00" --preview vista.png
```

- `--cols` es **cuántos personajes hay por fila** y `--rows` cuántas filas (1 si no lo pones). No hace falta que las
  celdas sean iguales: el script detecta cada personaje por su silueta, aunque estén muy pegados, con anchos distintos
  o con la bufanda tocando al siguiente. Si prefieres cortar en celdas iguales, añade `--split grid`.
- `--bg` es el color de fondo. Con `#00ff00` se quita el verde como un **croma**: borra también los puntitos sueltos
  entre las patas y limpia el tinte verde del contorno. Con `auto` (por defecto) usa el color más repetido en el borde
  de la hoja.
- `--preview vista.png` guarda una imagen con el resultado para que lo revises **antes de recompilar**.
- `--dry-run` comprueba todo sin escribir nada.
- Si queda un halo en el borde, sube `--trim 2` (o 3). Si faltan trozos del personaje o sobra verde, prueba a bajar o
  subir `--tol` (por defecto 40).
- Si es pixel art muy duro y al reescalar se ve borroso, usa `--resample nearest`.

El script ajusta todos los frames a la **misma escala** (la de tu personaje de reposo) y a la **misma línea del suelo**,
así que no saltan ni cambian de tamaño entre frames. Deja los archivos en `assets/<personaje>/<estado>/` y añade el
estado a `assets/characters.json`:

```json
"walking": { "folder": "peter-panda/walking", "pattern": "walking_{nn}.png", "frames": 6 }
```

Otros comandos:

```bash
python tools/import_sprites.py --list                      # qué estados propios tiene cada personaje
python tools/import_sprites.py peter_panda walking --remove  # vuelve a la animación de siempre
```

Después, `cargo build` (o `cargo test`, que comprueba que no falte ningún archivo) y abre la app: el estado nuevo se
usa solo. Si un archivo faltara o no cargase, ese estado vuelve a la animación de reposo en vez de fallar.

## Detalles útiles

- **Dirección:** la app voltea el sprite cuando camina hacia la izquierda, así que dibuja siempre hacia la derecha.
- **Velocidad:** al importar, cada animación recibe un ritmo (`"ms"`, milisegundos por frame) según lo que dura una
  vuelta típica de ese estado: andar ~0,9 s, reposo ~1,8 s, dormir ~3,6 s, contento ~1,1 s… Si te parece rápida o lenta,
  cambia solo el ritmo, sin volver a importar nada: más milisegundos = más lento.

  ```bash
  python tools/import_sprites.py peter_panda walking --ms 200
  ```

  También puedes fijarlo al importar con `--ms`. `python tools/import_sprites.py --list` muestra el ritmo de cada una.
  El `speed` del personaje en `characters.json` sigue multiplicándolo todo (útil para ir más lento con todas a la vez).
- **Balanceo por código:** los estados con frames dibujados aparte no llevan el rebote o balanceo que la app añade a los
  demás, porque se sumaría al movimiento del dibujo.
- **Tamaño:** los frames salen del mismo tamaño que el sprite de reposo del personaje (la mediana de las alturas de la
  animación iguala a la de `sprite_01.png`); no hace falta que la IA los genere a ese tamaño exacto. Si una animación
  se ve más pequeña o más grande que el resto, impórtala con `--scale 1.15` (un 15 % más grande) o `--scale 0.9`.
  Si el script avisa de que **redujo los frames para que cupieran**, es que la hoja trae un resto (una línea o una mota
  de otro color) o dos personajes pegados que no se separaron: mira `--preview` y prueba `--split grid`.
