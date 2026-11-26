// Datos ficticios: nunca se conecta a Tauri, a una IA ni a las cuentas del usuario.
const now = Date.parse("2026-11-26T10:00:00+01:00") / 1000;
const providers = [
  { id: "ollama", name: "Ollama", model: "qwen3.5:9b", default_model: "qwen3.5:9b", needs_key: false, key_source: "none" },
  { id: "gemini", name: "Google Gemini", model: "", default_model: "gemini-2.5-flash", needs_key: true, key_source: "none", key_url: "https://aistudio.google.com/apikey", env_var: "GEMINI_API_KEY" },
  { id: "claude", name: "Claude", model: "", default_model: "claude-sonnet-4-5", needs_key: true, key_source: "none" },
  { id: "openai", name: "OpenAI", model: "", default_model: "gpt-5", needs_key: true, key_source: "none" },
];
const tools = [
  ["list_files", "Lista los archivos de tus carpetas permitidas.", "safe"],
  ["read_file", "Lee un documento para responder tus preguntas.", "safe"],
  ["move_file", "Mueve archivos dentro de tus carpetas permitidas.", "confirm"],
  ["delete_file", "Envía archivos a la Papelera; no los elimina permanentemente.", "dangerous"],
  ["play_music", "Busca y reproduce música en tu aplicación.", "safe"],
  ["web_search", "Busca información en internet.", "safe"],
  ["set_reminder", "Crea un recordatorio con fecha y hora.", "safe"],
  ["look_at_screen", "Mira una ventana después de pedirte permiso.", "dangerous"],
  ["type_text", "Escribe en otra aplicación, con confirmación.", "dangerous"],
].map(([name, description, risk]) => ({ name, description, risk, enabled: name !== "type_text" }));
const settings = {
  assistant_name: "Ryan", provider: "ollama", providers, tools, character: "mochi",
  shortcut: "Super+Shift+K", shortcut_active: "Super+Shift+K",
  quick_shortcut: "Super+Shift+J", quick_shortcut_active: "Super+Shift+J",
  ollama_url: "http://localhost:11434", ollama_think: false,
  wander: true, look_at_cursor: true, music_reaction: true, sound_enabled: true,
  sound_volume: 35, battery_alerts: true, system_events: true, usage_comments: true,
  memory_enabled: true, computer_control: false, confirm_all: false, save_history: true,
  autostart: false, profile: "normal",
  profiles: [
    { id: "normal", name: "Normal", icon: "✦", description: "Tu compañero de siempre, listo para lo que necesites.", changes: [] },
    { id: "work", name: "Trabajo", icon: "▣", description: "Menos distracciones y respuestas más directas.", changes: ["Sin paseos mientras trabajas.", "Atajos para archivos y productividad.", "Un tono más breve y directo."] },
    { id: "rest", name: "Descanso", icon: "☕", description: "Baja el ritmo y tómate un respiro.", changes: ["Música, conversación y pequeños descansos."] },
    { id: "focus", name: "Concentración", icon: "◎", description: "Solo lo esencial para seguir con lo tuyo.", changes: ["Sin comentarios espontáneos.", "Sin sonidos ni paseos."] },
  ],
};
const reminders = [
  { id: 1, text: "Estirar las piernas y beber agua", due: now + 1200 },
  { id: 2, text: "Revisar la propuesta del proyecto", due: now + 5400 },
  { id: 3, text: "Llamar a la veterinaria", due: now + 10800 },
];
const tasks = [
  { id: 1, text: "Preparar la propuesta del proyecto", done: false, partial: true, note: "Falta revisar el presupuesto." },
  { id: 2, text: "Ordenar los documentos de la semana", done: false, partial: false },
  { id: 3, text: "Enviar el resumen de la reunión", done: true, partial: false },
];
const conversations = [
  { id: "c1", title: "Un escritorio más ordenado", updated: now - 600, count: 8, preview: "Organizamos Descargas por tipos y revisamos los archivos duplicados." },
  { id: "c2", title: "Música para trabajar", updated: now - 3600, count: 6, preview: "Una selección tranquila para acompañar la mañana." },
  { id: "c3", title: "Preparar la reunión del jueves", updated: now - 86400, count: 12, preview: "Ideas principales, próximos pasos y un recordatorio para revisar la propuesta." },
];
const activity = [
  { id: "a1", tool: "move_file", text: "✔ Moví propuesta.pdf a Documentos.", summary: "propuesta.pdf → Documentos\\Proyecto", args: { from: "C:\\Users\\Demo\\Downloads\\propuesta.pdf", to: "C:\\Users\\Demo\\Documents\\Proyecto\\propuesta.pdf" }, data: { to: "C:\\Users\\Demo\\Documents\\Proyecto\\propuesta.pdf" }, at: now - 240, duration_ms: 42, ok: true, confirmed: true, undo: "devolver el archivo a Descargas" },
  { id: "a2", tool: "set_reminder", text: "✔ Recordatorio creado.", summary: "Estirar las piernas y beber agua · hoy a las 10:20", args: { text: reminders[0].text, minutes: 20 }, data: {}, at: now - 600, duration_ms: 12, ok: true, confirmed: false, undo: "cancelar el recordatorio" },
  { id: "a3", tool: "play_music", text: "✔ Reproduciendo tu canción en Spotify.", summary: "Canción de ejemplo · Artista de ejemplo · Spotify", args: { query: "(canción)", source: "spotify" }, data: {}, at: now - 1800, duration_ms: 320, ok: true, confirmed: false },
];
const media = { title: "Canción de ejemplo", artist: "Artista de ejemplo", app: "Spotify", state: "paused", position: 87, duration: 272, can_previous: true, can_next: true, can_play_pause: true, artwork_key: null };
const fileMenu = {
  mode: "convert",
  files: [{ name: "foto-proyecto.png", type: "image", kind: "Imagen PNG", size: 2457600, folder: false }],
  ask: { enabled: true }, provider: { name: "Ollama", local: true },
  offers: [
    ...[["jpg", "JPG"], ["webp", "WebP"], ["bmp", "BMP"], ["gif", "GIF"], ["tiff", "TIFF"], ["ico", "ICO"], ["pdf", "PDF"], ["ocr", "Texto (OCR)"], ["zip", "Comprimir en ZIP"]].map(([id, label]) => ({ id, label, group: "convert" })),
    ...[["img_small", "Reducir peso"], ["img_half", "Mitad de tamaño"], ["img_strip", "Quitar metadatos"]].map(([id, label]) => ({ id, label, group: "optimize" })),
  ],
  output: { label: "Escritorio", path: "C:\\Users\\Demo\\Desktop", custom: false },
};
module.exports = { now, settings, reminders, tasks, conversations, activity, media, fileMenu };
