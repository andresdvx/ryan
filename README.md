# Ryan

Un asistente personal con IA que vive en tu escritorio de Windows como un compañero animado. Haz clic en él (o pulsa un atajo), abre el chat y pídele cosas: abrir programas, poner música, ordenar tus archivos, recordarte algo, buscar en internet o incluso mirar una ventana y explicarte qué muestra.

Está escrito en **Rust** con **Tauri 2**, y puede usar **Gemini, Claude, OpenAI** o **modelos locales con Ollama** (sin API key).

<p align="center">
  <img src="docs/images/chat.png" alt="Chat de Ryan reproduciendo música en Spotify" width="420">
</p>

> Las capturas de este README usan datos de ejemplo.

## Índice

- [Capturas](#capturas)
- [Qué puede hacer](#qué-puede-hacer)
- [Seguridad](#seguridad)
- [Requisitos](#requisitos)
- [Descarga](#descarga)
- [Configuración](#configuración)
- [Ejecución](#ejecución)
- [Uso](#uso)
- [Modelos locales con Ollama](#modelos-locales-con-ollama)
- [Dónde se guardan tus datos](#dónde-se-guardan-tus-datos)
- [Personalizar](#personalizar)
- [Estructura del proyecto](#estructura-del-proyecto)
- [Desarrollo](#desarrollo)
- [Problemas frecuentes](#problemas-frecuentes)

## Capturas

<table>
  <tr>
    <td align="center"><img src="docs/images/confirm.png" alt="Confirmación antes de borrar una carpeta" width="300"><br><sub>Las acciones de riesgo piden confirmación detallada</sub></td>
    <td align="center"><img src="docs/images/ai.png" alt="Ajustes de IA con Ollama" width="300"><br><sub>Elige proveedor de IA, incluido un modelo local</sub></td>
  </tr>
  <tr>
    <td align="center"><img src="docs/images/pet.png" alt="Ajustes del compañero" width="300"><br><sub>Nombre, avatar, comportamiento y memoria</sub></td>
    <td align="center"><img src="docs/images/security.png" alt="Ajustes de seguridad" width="300"><br><sub>Tú decides qué herramientas puede usar la IA</sub></td>
  </tr>
  <tr>
    <td align="center"><img src="docs/images/recordatorios.png" alt="Pestaña de recordatorios programados" width="300"><br><sub>Todos tus recordatorios, con cuenta atrás</sub></td>
    <td align="center"><img src="docs/images/modelo.png" alt="Selector de proveedor y modelo en la cabecera" width="300"><br><sub>Cambia de proveedor o de modelo desde la cabecera</sub></td>
  </tr>
  <tr>
    <td align="center"><img src="docs/images/comandos.png" alt="Paleta de comandos al escribir una barra" width="300"><br><sub>Escribe <code>/</code> y elige un comando</sub></td>
    <td align="center"><img src="docs/images/historial.png" alt="Historial de conversaciones con búsqueda" width="300"><br><sub>Historial: busca y retoma conversaciones</sub></td>
  </tr>
  <tr>
    <td align="center"><img src="docs/images/hoy.png" alt="Pestaña Hoy con el resumen del día" width="300"><br><sub>Hoy: música, recordatorios y estado del equipo</sub></td>
    <td align="center"><img src="docs/images/modos.png" alt="Modo Trabajo activo" width="300"><br><sub>Modos: Trabajo, Descanso, Concentración…</sub></td>
  </tr>
  <tr>
    <td align="center"><img src="docs/images/mirar-ventana.png" alt="Vista previa de la ventana antes de adjuntarla" width="300"><br><sub>Mirar una ventana: la ves antes de enviarla</sub></td>
    <td align="center"><img src="docs/images/editor-herramientas.png" alt="Editor de herramientas por categorías" width="300"><br><sub>Herramientas por categorías, con su nivel de riesgo</sub></td>
  </tr>
  <tr>
    <td align="center" colspan="2"><img src="docs/images/barra-rapida.png" alt="Barra rápida sobre el escritorio" width="420"><br><sub>Barra rápida: pide algo sin abrir el chat (<kbd>Win</kbd> + <kbd>Shift</kbd> + <kbd>J</kbd>)</sub></td>
  </tr>
</table>

### Personajes

Trae siete personajes; puedes [añadir los tuyos](#añadir-un-personaje).

<table>
  <tr>
    <td align="center"><img src="assets/character/standing/character-1.png" alt="Zen" height="96"><br>Zarina</td>
    <td align="center"><img src="assets/poring/standing/sprite_04.png" alt="Poring" height="96"><br>Slime</td>
    <td align="center"><img src="assets/pingu/sprite_01.png" alt="Pingu" height="96"><br>Pingu</td>
    <td align="center"><img src="assets/bandit/sprite_01.png" alt="Bandit" height="96"><br>Bandit</td>
    <td align="center"><img src="assets/mochi/sprite_01.png" alt="Mochi" height="96"><br>Mochi</td>
    <td align="center"><img src="assets/ellie/sprite_01.png" alt="Ellie" height="96"><br>Ellie</td>
    <td align="center"><img src="assets/drako/sprite_01.png" alt="Drako" height="96"><br>Drako</td>
  </tr>
</table>

## Qué puede hacer

| Área | Ejemplos |
| --- | --- |
| **Personaje** | Se anima según lo que hace (reposo, pensando, trabajando, contenta…), pasea por el escritorio, mira hacia el cursor, avisa con la batería baja y sugiere descansos. Su tono cambia con la hora, la batería y cuánto llevas trabajando, y reacciona cuando termina una descarga, conectas una memoria USB o se acerca un recordatorio. |
| **Chat** | Las respuestas se escriben en directo (con Ollama), una paleta de comandos al escribir `/`, y un selector de proveedor y modelo en la cabecera. Puedes **pegar una captura** (Ctrl + V) o pulsar 👁 para **mirar una ventana**: la ves antes de adjuntarla (con un modelo con visión). |
| **Barra rápida** | Un campo pequeño sobre el escritorio (atajo **Win + Shift + J**) para pedir algo sin abrir el chat. La respuesta aparece ahí, y si algo necesita tu permiso se abre el chat. Todo queda también en el historial. Toma los colores del personaje que elijas. |
| **Modos** | **Normal, Trabajo, Descanso y Concentración**: cambian a la vez cómo se comporta la mascota (paseos, comentarios, sonidos) y el tono de las respuestas, y los atajos de *Hoy*. Se aplican por encima de tus ajustes: al volver a Normal todo queda como lo tenías. |
| **Hoy** | Una pestaña con lo importante de un vistazo: lo que suena (con controles), tus próximos recordatorios, batería, memoria y espacio en disco, tu última conversación y atajos para pedir cosas con un clic. |
| **Historial** | Tus conversaciones se guardan en tu equipo. Puedes buscarlas (sin importar mayúsculas ni tildes), releerlas y **continuarlas**: Ryan recupera lo que hablasteis. Se puede apagar y borrar en cualquier momento. |
| **Archivos** | Crear, leer, buscar, mover, renombrar y **borrar enviando a la Papelera de Windows** (recuperable, nunca permanente). Copiar, comprimir y descomprimir `.zip`, y abrir carpetas o documentos. Organizar una carpeta por tipos y deshacerlo. Solo en Escritorio, Documentos y Descargas. |
| **Espacio en disco** | Saber qué ocupa espacio: los archivos más grandes, el tamaño de cada carpeta y los **archivos duplicados**, con cuánto se liberaría. Solo mide; si quieres borrar, te pregunta qué conservar y lo envía a la Papelera. También puede vaciar la Papelera, con confirmación detallada porque no tiene vuelta atrás. |
| **Programas y web** | Abrir las aplicaciones que tú permitas, abrir sitios web, buscar en internet con la búsqueda integrada de Gemini, Claude u OpenAI. |
| **Música y vídeo** | Reproducir una canción en la **app de escritorio de Spotify**, en YouTube Music o en YouTube. Reproductor en el chat con pausa, anterior y siguiente para lo que suene en Windows. |
| **Sistema** | Información del equipo (batería, memoria…), brillo, portapapeles, recordatorios con aviso sonoro. Una pestaña **Recordatorios** los lista con su cuenta atrás y permite crearlos y cancelarlos a mano. |
| **Memoria** | Recuerda lo que le pidas ("recuerda que me llamo Ana") entre sesiones. Nunca guarda contraseñas, claves ni datos de tarjetas. |
| **Ver la pantalla** | Con un modelo con visión (de momento, en Ollama) puede mirar **una ventana** y explicar lo que muestra, por ejemplo un mensaje de error. |
| **Control de otras apps** *(apagado por defecto)* | Escribir, pulsar teclas y pulsar botones en otras ventanas, con confirmación en cada acción. |

## Seguridad

La IA **no tiene terminal ni ejecuta comandos**: solo puede usar herramientas concretas, y todas pasan por el mismo camino: validación → permisos → confirmación → ejecución.

- **Tres niveles de riesgo.** Las acciones seguras se ejecutan solas; las delicadas piden confirmación; las peligrosas (borrar, escribir en otra app, mirar una ventana…) piden una confirmación detallada que te dice exactamente qué va a pasar.
- **Control de otras aplicaciones, apagado por defecto.** Se activa en *Ajustes > Seguridad*. Nunca actúa sobre terminales, administradores del sistema ni ventanas de contraseñas o pagos, y tiene un límite de 20 acciones por minuto.
- **Solo tus carpetas.** Las herramientas de archivos están limitadas a Escritorio, Documentos y Descargas, y no siguen enlaces que salgan de ellas.
- **Desactiva lo que no quieras.** Cada herramienta se puede apagar en *Ajustes > Seguridad*.
- **Tus claves están a salvo.** Las API keys se guardan en el Administrador de credenciales de Windows, nunca en un archivo del proyecto.
- **Lo que lee la IA son datos, no órdenes.** Los nombres de archivos, páginas web y resultados de herramientas no pueden darle instrucciones.

## Requisitos

- **Windows 10 u 11 de 64 bits.** Solo funciona en Windows.
- **[Rust](https://rustup.rs)** (instálalo con `rustup`; el proyecto se probó con Rust 1.99).
- **Herramientas de compilación de C++ de Visual Studio:** instala las [Build Tools](https://visualstudio.microsoft.com/es/visual-cpp-build-tools/) y marca la carga de trabajo *Desarrollo para el escritorio con C++*.
- **WebView2 Runtime.** Ya viene con Windows 11 y con casi todos los Windows 10; si no, [descárgalo](https://developer.microsoft.com/microsoft-edge/webview2/).
- **Git** (opcional, para clonar el repositorio).
- **Una IA:** una API key de [Gemini](https://aistudio.google.com/apikey), [Claude](https://console.anthropic.com/settings/keys) u [OpenAI](https://platform.openai.com/api-keys), o [Ollama](https://ollama.com/download) para usar modelos locales sin clave.

## Descarga

Con Git:

```bash
git clone https://github.com/andresdvx/ryan.git
cd ryan
```

### Un compañero con carácter

- **Un chat con personalidad:** cada amigo tiñe el chat con sus propios colores (campos `accent` y `accent2` en `assets/characters.json`), aparece animado en la cabecera según lo que está haciendo y pone su carita junto a sus mensajes. Saluda según la hora del día y propone ideas para empezar.
- **Ánimo:** antes de cada respuesta, la IA recibe una pista de estilo según la hora (más suave de madrugada, más animado por la mañana), la batería (más breve si queda poca) y cuánto llevas sin una pausa (te sugiere descansar). Solo cambia el tono, nunca lo que hace ni sus reglas. La duración de tu racha de trabajo solo se usa si dejas activados los comentarios sobre el uso del equipo (*Ajustes > Compañero*).
- **Escucha música contigo:** cuando suena música o un vídeo en cualquier aplicación que use los controles multimedia de Windows (Spotify, Apple Music, el navegador, VLC…), el personaje se pone unos auriculares del color de su acento, cabecea al ritmo, le salen notas musicales y se queda despierto disfrutándola. Al parar la música se los quita. Se puede apagar en *Ajustes > Compañero*.
- **Avisos del equipo** (*Ajustes > Avisos*): te avisa cuando termina una descarga en la carpeta Descargas o conectas una memoria USB. Solo mira los nombres de los archivos, nunca su contenido.
- **Recordatorios:** además del aviso al vencer, te avisa 10 minutos antes (útil para reuniones).

## Configuración

No hay archivos de configuración que editar a mano: todo se hace desde la propia aplicación, con el engranaje ⚙️ de la cabecera del chat (junto al modelo en uso).

### 1. Elige tu IA

En **Ajustes > IA** elige el proveedor (por defecto, Gemini) y, si quieres, un modelo concreto. Si lo dejas vacío usa el predeterminado de ese proveedor.

**Con API key (Gemini, Claude, OpenAI)**

1. Consigue una clave en la página del proveedor (enlaces en [Requisitos](#requisitos)).
2. Pégala en *Ajustes > IA > API key* y pulsa *Guardar*. Se guarda en el Administrador de credenciales de Windows.
3. Alternativa para desarrollo: define la variable de entorno `GEMINI_API_KEY`, `ANTHROPIC_API_KEY` u `OPENAI_API_KEY` antes de ejecutar. Si hay una clave guardada, tiene prioridad sobre la variable.

También puedes escribir `/key TU_CLAVE` en el chat (guarda la clave del proveedor activo) o `/forget` para borrarla.

**Sin API key (modelo local)**

Elige *Ollama (modelo local)*; mira [Modelos locales con Ollama](#modelos-locales-con-ollama).

### 2. Personaliza lo demás

| Pestaña | Qué configuras |
| --- | --- |
| **Compañero** | Nombre del asistente (la IA lo conoce), avatar, si pasea y mira al cursor, comentarios sobre tu uso del equipo y memoria. |
| **IA** | Proveedor, modelo, API key o dirección de Ollama. |
| **Avisos** | Sonidos y volumen, avisos de batería, y avisos de descargas terminadas y memorias USB conectadas. |
| **Sistema** | Atajos de teclado (chat y barra rápida), iniciar con Windows y qué aplicaciones puede abrir la IA, con buscador y botones para activar o desactivar varias a la vez. |
| **Seguridad** | Confirmación en todas las acciones, control de otras aplicaciones y qué herramientas están activas: agrupadas por categoría, con buscador, el nivel de riesgo de cada una y botones para activar o desactivar todo un grupo. |

El atajo para abrir el chat es **Win + Shift + K** (si otro programa lo usa, prueba con **Ctrl + Alt + K**) y el de la barra rápida, **Win + Shift + J**; ambos se cambian en *Ajustes > Sistema*.

## Ejecución

Desde la carpeta del proyecto:

```bash
cargo run -p ryan
```

La primera vez descarga y compila las dependencias, y **puede tardar varios minutos**. Las siguientes son rápidas. Para una versión optimizada:

```bash
cargo run -p ryan --release
```

El ejecutable queda en `target\release\ryan.exe` y puedes abrirlo con doble clic.

Al arrancar aparece tu compañero en una esquina del escritorio y un icono en la bandeja del sistema. Haz clic en él, pulsa el atajo o usa el icono de la bandeja para abrir el chat. Solo se abre una copia a la vez. Para cerrarlo, usa el icono de la bandeja > **Salir**.

## Uso

Abre el chat y escribe con naturalidad. Algunos ejemplos:

- "Pon *Do I Wanna Know* de Arctic Monkeys en Spotify."
- "Busca en YouTube un vídeo de recetas de pasta y reprodúcelo."
- "Crea una carpeta llamada Fotos en el escritorio."
- "Organiza mi carpeta de Descargas." / "Deshaz la última organización."
- "Borra la carpeta Pruebas del escritorio." *(te pedirá confirmación y la enviará a la Papelera)*
- "¿Qué ocupa más espacio en mis Descargas?" / "Busca archivos duplicados en el escritorio."
- "Comprime la carpeta Proyecto en un zip." / "Descomprime el zip de Descargas."
- "Haz una copia del informe." / "Abre la carpeta Descargas."
- "Recuérdame estirar las piernas en 30 minutos."
- "¿Qué noticias hay hoy sobre…?"
- "Recuerda que mi comida favorita es la pizza."
- "Mira esta ventana y dime qué error muestra." *(con un modelo con visión)*

Comandos del chat:

| Comando | Qué hace |
| --- | --- |
| `/new` | Empieza una conversación nueva. |
| `/modelo` | Abre el selector de proveedor y modelo de la cabecera. |
| `/tools` | Lista las herramientas disponibles. |
| `/key TU_CLAVE` | Guarda la API key del proveedor activo. |
| `/forget` | Borra la API key guardada del proveedor activo. |

**Música:** para Spotify necesitas la **aplicación de escritorio** con la sesión iniciada (no sirve Spotify Web). YouTube y YouTube Music se abren en tu navegador predeterminado.

## Modelos locales con Ollama

Con [Ollama](https://ollama.com/download) la IA corre en tu equipo: sin API key, sin coste y sin que tus mensajes salgan de él.

1. Instala Ollama y descarga un modelo:

   ```bash
   ollama pull qwen3:8b
   ```

2. En *Ajustes > IA* elige **Ollama (modelo local)**.
3. Pulsa **Probar y buscar modelos**: comprueba la conexión y te sugiere los que tienes descargados. Escribe el nombre en *Modelo* y guarda. La dirección por defecto es `http://localhost:11434`.

Qué modelo elegir (Ryan necesita que **admita herramientas**):

| Modelo | Para qué | Tamaño aprox. |
| --- | --- | --- |
| `qwen3:8b` | Uso diario. Es el predeterminado y cabe en una GPU de 8 GB. | 5 GB |
| `qwen3:4b` | Equipos más modestos. Algo menos fiable con herramientas. | 2,5 GB |
| `qwen3.5:9b` | **Tiene visión**: permite que mire una ventana con `look_at_screen`. | 6,6 GB |
| `llama3.1:8b` | Alternativa con buen soporte de herramientas. | 5 GB |

Limitaciones con Ollama:

- No tiene búsqueda web integrada, así que la herramienta de búsqueda no funciona con él.
- Los modelos pequeños se equivocan más al elegir herramientas que Gemini, Claude u OpenAI.
- `deepseek-r1` en su versión base no admite herramientas, así que Ollama lo rechaza.
- La primera respuesta tarda más porque el modelo se carga en memoria.
- Los modelos con razonamiento (Qwen3, Qwen3.5…) piensan en silencio antes de escribir, y eso puede tardar decenas de segundos. Por eso está **desactivado por defecto**: con `qwen3.5:9b` el primer texto aparece en unos 2 s en lugar de unos 30 s. Puedes activarlo en *Ajustes > IA* o en el selector de modelo de la cabecera si prefieres respuestas más meditadas.

## Dónde se guardan tus datos

| Dato | Dónde |
| --- | --- |
| Ajustes (atajo, avatar, modelo elegido…) | `%APPDATA%\ZenPet\settings.json` |
| Memoria, recordatorios y registro para deshacer | `%APPDATA%\ZenPet\` |
| Historial de conversaciones (un archivo por conversación) | `%APPDATA%\ZenPet\conversations\` |
| API keys | Administrador de credenciales de Windows (`zen-pet`) |

El historial guarda solo lo que se ve en el chat (tus mensajes y las respuestas), nunca imágenes ni claves: un mensaje que parece contener una contraseña o una clave no se guarda. Se puede apagar y borrar desde la pestaña *Historial*, se limita a las 200 conversaciones más recientes y no sale de tu equipo.

Lo que escribes en el chat se envía al proveedor de IA que hayas elegido (con Ollama no sale de tu equipo). Las imágenes que pegas o adjuntas con 👁 solo se envían con el mensaje que las acompaña, se comprueban antes (solo PNG o JPEG reales, hasta 3 por mensaje) y no se guardan en el historial. Una captura de ventana que pide la IA solo se envía cuando confirmas cada vez, no se guarda en disco y solo se conserva la más reciente durante la conversación.

## Personalizar

### Añadir un personaje

1. Crea una carpeta en `assets/` con los fotogramas del personaje (PNG numerados).
2. Añádelo a `assets/characters.json`:

   ```json
   {
     "id": "mipersonaje",
     "name": "Mi personaje",
     "folder": "mipersonaje",
     "pattern": "sprite_{nn}.png",
     "frames": 8,
     "preview_frame": 1,
     "speed": 1.0
   }
   ```

   `{n}` es el número sin ceros (1, 2…) y `{nn}` con dos cifras (01, 02…). Se compila de nuevo y aparece en *Ajustes > Compañero > Avatar*. Para que se ponga auriculares cuando suena música, añade `"headphones": { "x": 50, "y": 34, "width": 60 }` (centro y ancho de los auriculares, en % del sprite) y genera su imagen con `python tools/headphones.py`, que usa el color `accent` del personaje y escribe `assets/headphones/<id>.png`. Un estado `"listening"` opcional le da la expresión con la que disfruta la música. El archivo explica además cómo darle una animación propia a cada estado: con otra carpeta de frames o, si el personaje ya tiene las poses que hacen falta, con una lista de frames propios. Pingu lo hace así:

   ```json
   "states": { "sleeping": { "sequence": [3, 10, 11, 10] }, "thinking": { "sequence": [1, 5, 6, 1, 8, 12] } }
   ```

### Añadir una herramienta

Cada herramienta implementa el trait `Tool` de `zen-core` (nombre, esquema de parámetros, nivel de riesgo, validación, vista previa y ejecución) y se registra en `crates/zen-desktop/src/lib.rs`. El resto, incluidos los permisos y la confirmación, lo gestiona el sistema.

## Estructura del proyecto

```text
assets/            Sprites y manifiesto de personajes (assets/characters.json)
crates/
  zen-core/        Núcleo: eventos, herramientas, permisos, ejecutor y bucle del asistente
  zen-character/   Máquina de estados y comportamiento del compañero
  zen-desktop/     Herramientas de Windows: archivos, apps, música, ventanas, sistema…
  zen-ai/          Proveedores de IA: Gemini, Claude, OpenAI y Ollama
src-tauri/         Aplicación Tauri 2: ventanas, ajustes, bandeja, atajo y comandos
ui/                Interfaz (HTML, CSS y JavaScript sin dependencias)
docs/images/       Capturas de este README
plan.md            Especificación original del proyecto
```

`ui/assets/` se genera sola al compilar a partir de `assets/`; no hay que tocarla ni subirla.

## Desarrollo

```bash
cargo test --workspace
```

Algunas pruebas necesitan el sistema real (Spotify, la Papelera, Ollama, una ventana abierta) y están marcadas como ignoradas. Se ejecutan a propósito, por ejemplo:

```bash
cargo test -p zen-desktop -- --ignored --nocapture papelera_real
cargo test -p zen-ai -- --ignored --nocapture ollama_real
```

## Problemas frecuentes

**`Access is denied` al compilar.** Ryan sigue abierto y Windows no deja reemplazar el programa. Ciérralo desde el icono de la bandeja y vuelve a compilar.

**"Para hablar conmigo necesito tu API key".** Falta la clave del proveedor elegido: guárdala en *Ajustes > IA*, o cambia a Ollama.

**"Ollama no responde".** Comprueba que Ollama esté abierto y que la dirección de *Ajustes > IA* sea correcta (por defecto, `http://localhost:11434`).

**El modelo no puede ver imágenes.** Para `look_at_screen` usa un modelo con visión, como `qwen3.5:9b`.

**El atajo no abre el chat.** Otro programa lo está usando. Cámbialo en *Ajustes > Sistema* o usa el icono de la bandeja.

**No reproduce en Spotify.** Necesita la aplicación de escritorio abierta con la sesión iniciada. Si Spotify cambia su interfaz, la herramienta puede necesitar un ajuste.

**El navegador no abre la pestaña.** YouTube y YouTube Music se abren con el navegador predeterminado de Windows.

## Licencia

Pendiente de elegir. Añade un archivo `LICENSE` antes de aceptar contribuciones o de compartir el proyecto.
