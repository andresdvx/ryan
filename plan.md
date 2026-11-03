Quiero que actúes como un **Arquitecto de Software Senior especializado en Rust, aplicaciones nativas de Windows, inteligencia artificial y sistemas de automatización de escritorio**.

Tu objetivo es ayudarme a diseñar y construir un proyecto llamado **Ryan**, un asistente personal con IA que viva visualmente en el escritorio de Windows como una mascota o personaje animado.

# 1. VISIÓN DEL PROYECTO

Quiero crear una aplicación nativa para Windows que combine:

- Una mascota virtual animada.
- Un asistente personal impulsado por IA.
- Un sistema de automatización seguro para Windows.
- Un panel de chat.
- Animaciones y comportamientos que hagan parecer que el personaje vive en mi escritorio.

El personaje debe poder permanecer visible en el escritorio y comportarse como si el escritorio fuera su casa.

Ejemplos de comportamiento:

- Permanecer en estado idle.
- Caminar por determinadas zonas del escritorio.
- Dormir cuando no está siendo utilizado.
- Mirar o reaccionar al cursor.
- Reaccionar cuando el usuario hace click sobre él.
- Mostrar estados emocionales.
- Pensar mientras procesa una solicitud.
- Trabajar mientras ejecuta una acción.
- Celebrar o reaccionar cuando termina correctamente una tarea.

El usuario debe poder hacer click sobre el personaje o utilizar un atajo global para abrir un panel de chat.

# 2. INTERACCIÓN CON EL USUARIO

El asistente debe tener un panel de chat donde el usuario pueda escribir solicitudes en lenguaje natural.

Ejemplos:

"Pon música de Arctic Monkeys."

"Crea una carpeta llamada Proyecto."

"Crea un archivo README.md."

"Organiza los accesos directos de mi escritorio."

"Abre Spotify."

"Abre Visual Studio Code."

"Crea una carpeta para este nuevo proyecto."

La IA debe interpretar la intención del usuario y determinar qué herramientas disponibles puede utilizar.

# 3. PRINCIPIO FUNDAMENTAL DE SEGURIDAD

La IA NO debe tener acceso directo e ilimitado al sistema operativo.

NO quiero un sistema donde el modelo pueda ejecutar libremente:

- PowerShell.
- CMD.
- Scripts arbitrarios.
- Comandos del sistema sin restricciones.

En su lugar, quiero una arquitectura basada en herramientas explícitas.

Flujo deseado:

Usuario
↓
IA interpreta la solicitud
↓
La IA selecciona una herramienta permitida
↓
El sistema valida argumentos y permisos
↓
Se determina si la acción requiere confirmación
↓
Se ejecuta la acción
↓
Se informa el resultado al usuario

Ejemplo:

Usuario:
"Crea una carpeta llamada Proyecto Zen en el escritorio."

La IA debería solicitar conceptualmente una herramienta:

create_folder(
    location = Desktop,
    name = "Proyecto Zen"
)

El programa debe validar los argumentos antes de ejecutar cualquier acción.

# 4. TECNOLOGÍA PRINCIPAL

NO quiero utilizar Node.js ni Electron como backend principal.

Quiero priorizar:

- Rust.
- Rendimiento.
- Bajo consumo de memoria.
- Seguridad.
- Aplicación nativa para Windows.

Investiga y recomienda una arquitectura moderna basada preferiblemente en:

- Rust.
- Tauri u otra alternativa apropiada para la interfaz.
- Windows API cuando sea necesario.
- Un frontend ligero para el panel de chat.

Antes de recomendar dependencias o frameworks, explica claramente:

1. Qué problema resuelve cada tecnología.
2. Por qué fue elegida.
3. Qué alternativas existen.
4. Qué ventajas y desventajas tiene.

Evita agregar dependencias innecesarias.

# 5. ARQUITECTURA DEL SISTEMA

Quiero una arquitectura modular.

Propón una estructura similar conceptualmente a:

ryan/

├── src/
│
├── core/
│   ├── assistant/
│   ├── ai/
│   ├── tools/
│   ├── permissions/
│   └── events/
│
├── desktop/
│   ├── windows/
│   ├── shortcuts/
│   └── applications/
│
├── character/
│   ├── state/
│   ├── animations/
│   ├── behavior/
│   └── interaction/
│
├── ui/
│   ├── chat/
│   └── settings/
│
└── assets/

La estructura exacta puede cambiar si existe una mejor arquitectura.

Explica las responsabilidades de cada módulo.

# 6. SISTEMA DE HERRAMIENTAS

Diseña un sistema extensible de herramientas.

Ejemplos iniciales:

- create_folder
- create_file
- open_application
- open_website
- organize_desktop
- list_files
- move_file
- play_music

Cada herramienta debe tener:

- Nombre.
- Descripción.
- Esquema de parámetros.
- Validación.
- Nivel de riesgo.
- Política de confirmación.
- Ejecución.
- Resultado estructurado.

Diseña una abstracción idiomática en Rust.

No escribas código innecesariamente complejo.

# 7. NIVELES DE RIESGO

Implementa un sistema conceptual de permisos.

Nivel SAFE:

Acciones que pueden ejecutarse normalmente.

Ejemplos:

- Abrir una aplicación.
- Crear una carpeta.
- Crear un archivo.

Nivel CONFIRM:

Acciones que requieren confirmación del usuario.

Ejemplos:

- Mover múltiples archivos.
- Reorganizar el escritorio.
- Cerrar aplicaciones.

Nivel DANGEROUS:

Acciones que siempre requieren una confirmación explícita y detallada.

Ejemplos:

- Eliminar archivos.
- Modificar configuraciones importantes.
- Ejecutar comandos arbitrarios.

La IA nunca debe poder ignorar estas políticas.

# 8. PERSONAJE Y ANIMACIONES

Diseña una arquitectura para una mascota virtual.

El personaje debe funcionar mediante una máquina de estados.

Estados iniciales:

IDLE
↓
THINKING
↓
WORKING
↓
SUCCESS
↓
IDLE

Estados adicionales:

WALKING
SLEEPING
HAPPY
SURPRISED
INTERACTING

La transición entre estados debe estar controlada por eventos.

Ejemplo:

UserMessageReceived
→ THINKING

ToolExecutionStarted
→ WORKING

ToolExecutionSuccess
→ HAPPY o SUCCESS

ToolExecutionFailed
→ ERROR o SURPRISED

Diseña una máquina de estados clara y extensible.

# 9. COMPORTAMIENTO DE LA MASCOTA

La mascota no debe ser únicamente una animación estática.

Debe tener un sistema de comportamiento.

Ejemplos:

- Después de cierto tiempo puede cambiar de idle.
- Puede dormir después de un período largo de inactividad.
- Puede reaccionar al cursor.
- Puede ejecutar animaciones aleatorias.
- Puede reaccionar a eventos del sistema.
- Puede mostrar visualmente que está pensando.
- Puede acompañar visualmente ciertas acciones.

El comportamiento debe ser desacoplado del sistema de IA.

# 10. PANEL DE CHAT

El panel debe abrirse mediante:

- Click sobre el personaje.
- Atajo global configurable.

El panel debe incluir:

- Historial de conversación.
- Campo de texto.
- Estado del asistente.
- Confirmaciones de acciones.
- Visualización de tareas ejecutándose.

Ejemplo:

🐱 Estoy organizando tus archivos...

████████░░░░░░

No quiero que la IA parezca ejecutar acciones invisiblemente.

El usuario debe poder ver claramente:

- Qué está haciendo.
- Qué herramienta utilizará.
- Qué archivos serán afectados.
- Cuándo terminó.

# 11. SISTEMA DE EVENTOS

Diseña un sistema interno de eventos.

Ejemplos:

CharacterClicked
ChatOpened
UserMessageReceived
AssistantThinking
ToolRequested
ToolConfirmationRequired
ToolStarted
ToolCompleted
ToolFailed

Estos eventos deben permitir conectar:

- IA.
- UI.
- Personaje.
- Sistema de herramientas.

Evita un acoplamiento excesivo entre módulos.

# 12. IA Y TOOL CALLING

La IA debe funcionar principalmente como:

- Intérprete de lenguaje natural.
- Planificador de acciones.
- Seleccionador de herramientas.

La lógica de seguridad debe permanecer fuera del modelo.

La IA puede solicitar:

create_folder(...)

Pero el programa es quien decide:

- Si los argumentos son válidos.
- Si la acción está permitida.
- Si requiere confirmación.
- Si puede ejecutarse.

Diseña una interfaz clara entre:

AI Layer
↓
Tool Registry
↓
Permission System
↓
Tool Executor

# 13. CONFIGURACIÓN

La aplicación debe permitir configurar:

- API de IA.
- Atajo global.
- Personaje seleccionado.
- Volumen.
- Comportamientos.
- Herramientas permitidas.
- Confirmaciones.
- Inicio automático con Windows.

# 14. OBJETIVO DEL MVP

El primer MVP debe ser pequeño y funcional.

Debe incluir:

1. Una aplicación que pueda ejecutarse en Windows.
2. Un personaje visible.
3. Una animación idle.
4. Click sobre el personaje.
5. Panel de chat.
6. Conexión con IA.
7. Tool calling.
8. Crear carpetas.
9. Crear archivos.
10. Abrir aplicaciones.
11. Confirmación para acciones sensibles.

NO implementar inicialmente:

- Control total del sistema.
- Automatización compleja.
- Decenas de herramientas.
- Memoria avanzada.
- Voz.
- Visión por cámara.
- Personajes extremadamente complejos.

Primero construir una base sólida.

# 15. FORMA DE TRABAJO

Quiero que trabajes conmigo de forma incremental.

NO generes todo el proyecto de una sola vez.

Para cada fase:

1. Explica el objetivo.
2. Explica la arquitectura.
3. Propón los archivos necesarios.
4. Explica las decisiones técnicas.
5. Implementa únicamente lo necesario.
6. Explica cómo ejecutar y probar.
7. Espera mi confirmación antes de avanzar a la siguiente fase.

Cuando escribas código:

- Utiliza Rust idiomático.
- Prioriza claridad.
- Evita abstracciones prematuras.
- Agrega comentarios únicamente donde aporten valor.
- Separa responsabilidades.
- No crees código ficticio.
- No asumas APIs inexistentes.
- Mantén el proyecto compilable.

# 16. RESULTADO ESPERADO

El resultado final debe ser una aplicación para Windows donde:

Un personaje animado vive visualmente en el escritorio.

↓

El usuario interactúa con él.

↓

Se abre un chat.

↓

El usuario escribe una solicitud.

↓

La IA interpreta la intención.

↓

Selecciona una herramienta permitida.

↓

El sistema valida la acción.

↓

El usuario confirma si es necesario.

↓

La acción se ejecuta.

↓

El personaje muestra una animación correspondiente.

↓

El usuario recibe el resultado.

Comienza proponiendo la arquitectura definitiva y el plan de implementación por fases.

NO escribas todo el código todavía.

puedes instalar skills que recomiendes, tienes permiso