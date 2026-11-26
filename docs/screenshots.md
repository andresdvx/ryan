# Capturas del README

Las imágenes de `docs/images/` se renderizan con **la UI real** de `ui/`, no con maquetas ni imágenes generadas por IA. `tools/docs-fixtures.cjs` contiene únicamente datos ficticios.

## Regenerar en Windows

Necesitas Node.js, npm y Microsoft Edge. Desde la raíz del repositorio, en PowerShell:

```powershell
npm install --prefix "$env:LOCALAPPDATA\Temp\opencode\ryan-docs" playwright --no-audit --no-fund
$env:NODE_PATH = "$env:LOCALAPPDATA\Temp\opencode\ryan-docs\node_modules"
node tools/capture-readme.cjs
```

Si usas Chrome, establece `$env:DOCS_BROWSER = "chrome"` antes de ejecutar el script.

El script:

- Sirve la carpeta `ui/` temporalmente en `127.0.0.1`, en un puerto libre.
- Si todavía no existe `ui/assets/`, usa los recursos originales de `assets/`; no es necesario compilar Rust para generar las capturas.
- Sustituye Tauri por respuestas de demostración antes de cargar los scripts.
- Usa los componentes, renderizadores y formularios existentes.
- Fija la fecha, el idioma, la zona horaria y el personaje para dar consistencia.
- Captura el chat amplio a 880 × 720, el compacto a 620 × 720, la barra rápida a 640 px, la barra de arrastre a 340 px y el menú de conversión a 380 px. Los PNG se guardan a una escala de 1,5 para mejorar la legibilidad.
- Genera escenas de funcionalidades: arrastrar, analizar, resumir y convertir archivos; escuchar música; gestionar tareas y recordatorios; organizar el Escritorio y carpetas; explicar una ventana; consultar el estado del equipo y usar la barra rápida.
- Incluye `enfoque.png` y `rutinas.png`, renderizados desde los nuevos componentes de Hoy con el temporizador y una rutina de demostración.
- Conserva también las capturas complementarias de ajustes, historial y seguridad en `docs/images/`, aunque ya no ocupan la galería principal del README.
- Cierra el navegador y el servidor al terminar. Falla si encuentra errores JavaScript.

No inicia Ryan, no lee su configuración, no accede a sus claves ni ejecuta acciones sobre archivos, música o ventanas reales. La vista previa de Bloc de notas se crea con un documento ficticio. Los análisis, resúmenes y resultados de herramientas son respuestas simuladas que se muestran con los renderizadores reales. La conversión utiliza el formulario y botón reales, pero su resultado también es simulado: no se convierte ningún archivo.

El ejemplo musical usa «Reproduce (canción) en Spotify»: `(canción)` es un marcador que el usuario debe sustituir, no un título concreto.

## Al cambiar la interfaz

1. Actualiza los escenarios si cambian los controles o los formatos de respuesta.
2. Ejecuta el script y revisa los PNG, especialmente los paneles con desplazamiento.
3. Comprueba que la descripción y el texto alternativo del README correspondan a cada captura.
4. Incluye el README y las imágenes actualizadas en el mismo cambio.

## Prueba de interfaz de productividad

Con las mismas dependencias y `NODE_PATH`, ejecuta:

```powershell
node tools/test-productivity.cjs
```

Comprueba el formulario de recurrencia, los controles del temporizador, la creación y edición de rutinas, la revisión antes de ejecutar, la cancelación y los anchos de 380, 620 y 880 px. Usa una API simulada; la persistencia y los permisos reales se prueban con `cargo test --workspace`.
