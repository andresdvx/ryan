const { invoke } = window.__TAURI__.core;
const appWindow = window.__TAURI__.window.getCurrentWindow();

const history = document.getElementById("history");
const form = document.getElementById("form");
const input = document.getElementById("input");
const statusEl = document.getElementById("status");

// Indicador de "pensando / trabajando" que va siempre al final del historial.
let typingEl = null;
let busyState = null; // "thinking" | "working" | null
let pendingConfirmations = 0;
// Burbuja de la respuesta que se está escribiendo en directo (null si no hay ninguna).
let streamEl = null;
let streamText = "";

function addToHistory(el) {
  history.insertBefore(el, typingEl); // con typingEl nulo, equivale a añadir al final
  history.scrollTop = history.scrollHeight;
}

const URL_PATTERN = /https:\/\/[^\s<>"')\]]+/g;

// Los enlaces https del texto se pueden pulsar: se abren en el navegador a través de la app, que valida cada
// dirección (la ventana del chat nunca navega por sí misma). Todo se construye con nodos, sin HTML.
function appendWithLinks(el, text) {
  let last = 0;
  for (const match of text.matchAll(URL_PATTERN)) {
    // La puntuación final de una frase («…/b,») no es parte de la dirección.
    const url = match[0].replace(/[.,;:!?]+$/, "");
    el.append(text.slice(last, match.index));
    const link = document.createElement("a");
    link.className = "link";
    link.href = "#";
    link.textContent = url;
    link.addEventListener("click", (event) => {
      event.preventDefault();
      invoke("open_link", { url }).catch((error) => addMessage("assistant", `No pude abrir el enlace: ${error}`));
    });
    el.append(link);
    last = match.index + url.length;
  }
  el.append(text.slice(last));
}

const openLink = (url) =>
  invoke("open_link", { url }).catch((error) => addMessage("assistant", `No pude abrir el enlace: ${error}`));

async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // Sin permiso del portapapeles del navegador: se usa el método clásico.
    const area = document.createElement("textarea");
    area.value = text;
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.append(area);
    area.select();
    document.execCommand("copy");
    area.remove();
  }
}

function addCopyButton(el, text) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "copy";
  button.title = "Copiar mensaje";
  button.setAttribute("aria-label", "Copiar mensaje");
  button.textContent = "Copiar";
  button.addEventListener("click", async () => {
    await copyText(text);
    button.textContent = "Copiado ✓";
    setTimeout(() => (button.textContent = "Copiar"), 1400);
  });
  el.append(button);
}

// Los avisos con icono (✔ ✖ ⏰ 🔋) se muestran tal cual; el resto de lo que dice el asistente admite formato.
const PLAIN_PREFIX = /^(✔|✖|⏰|🔋|🔌|⚠)/u;

// `at` (segundos desde 1970) permite mostrar la hora original al recuperar una conversación guardada.
function buildMessage(role, text, at) {
  const el = document.createElement("div");
  el.className = `msg ${role}`;
  if (role.startsWith("assistant") && !PLAIN_PREFIX.test(text) && window.renderRich) {
    el.classList.add("rich");
    window.renderRich(el, text, openLink);
  } else if (role.startsWith("assistant")) {
    appendWithLinks(el, text);
  } else {
    el.textContent = text;
  }
  addCopyButton(el, text);
  const time = document.createElement("span");
  time.className = "time";
  time.textContent = new Date(at ? at * 1000 : Date.now()).toLocaleTimeString("es", { hour: "2-digit", minute: "2-digit" });
  el.append(time);
  return el;
}

function addMessage(role, text) {
  addToHistory(buildMessage(role, text));
}

// Un mensaje del usuario con las miniaturas de las imágenes que adjuntó.
function addUserMessage(text, images = []) {
  const el = buildMessage("user", text);
  if (images.length) {
    const strip = document.createElement("div");
    strip.className = "attached";
    for (const image of images) {
      const thumb = document.createElement("img");
      thumb.src = `data:${image.mime};base64,${image.data}`;
      thumb.alt = image.label || "Imagen adjunta";
      strip.append(thumb);
    }
    el.insertBefore(strip, el.firstChild);
  }
  addToHistory(el);
}

// Frases para esperar: cambian en cada espera, pero no a mitad de ella.
const WAITING = {
  thinking: ["Pensando…", "Déjame ver…", "Mmm, a ver…", "Un momentito…"],
  working: ["Manos a la obra…", "En eso estoy…", "Casi casi…", "Trabajando en ello…"],
};
let phrase = null;

function typingPhrase() {
  if (!phrase || phrase.state !== busyState) {
    const options = WAITING[busyState] ?? WAITING.thinking;
    phrase = { state: busyState, text: options[Math.floor(Math.random() * options.length)] };
  }
  return phrase.text;
}

function renderTyping() {
  // Mientras se espera la respuesta del usuario a una confirmación no se aparenta estar ocupado.
  // Con texto escribiéndose ya no hacen falta los puntitos de espera.
  const show = busyState !== null && pendingConfirmations === 0 && !streamEl;
  statusEl.dataset.busy = String(show);
  if (!show) {
    typingEl?.remove();
    typingEl = null;
    return;
  }
  if (!typingEl) {
    typingEl = document.createElement("div");
    typingEl.className = "msg assistant typing";
    typingEl.setAttribute("role", "status");
    const dots = document.createElement("span");
    dots.className = "dots";
    dots.append(...[0, 1, 2].map(() => document.createElement("i")));
    const label = document.createElement("span");
    label.className = "label";
    const steps = document.createElement("span");
    steps.className = "steps";
    steps.hidden = true;
    typingEl.append(dots, label, steps);
    history.appendChild(typingEl);
  }
  typingEl.querySelector(".label").textContent = typingPhrase();
  const steps = typingEl.querySelector(".steps");
  steps.textContent = progressText(taskProgress);
  steps.hidden = !steps.textContent;
  history.scrollTop = history.scrollHeight;
}

// Avance de una tarea de varios pasos: «██░░ 2/4 · mover archivo». Con un solo paso solo se nombra la herramienta.
let taskProgress = null;
const BAR_WIDTH = 10;

function progressText(progress) {
  if (!progress) return "";
  const tool = progress.tool.replace(/_/g, " ");
  if (progress.total <= 1) return tool;
  const filled = Math.round((Math.max(0, progress.step - 1) / progress.total) * BAR_WIDTH);
  return `${"█".repeat(filled)}${"░".repeat(BAR_WIDTH - filled)} ${progress.step}/${progress.total} · ${tool}`;
}

function setBusy(state) {
  busyState = state;
  renderTyping();
}

window.__TAURI__.event.listen("task-progress", (e) => {
  taskProgress = e.payload;
  renderTyping();
});

// Lo que escribiste, para recuperarlo con las flechas ↑ ↓ (las claves nunca se guardan).
const sent = (() => {
  try {
    const saved = JSON.parse(localStorage.getItem("zen-sent") ?? "[]");
    return Array.isArray(saved) ? saved.filter((t) => typeof t === "string").slice(-50) : [];
  } catch {
    return [];
  }
})();
let recallIndex = null; // posición en `sent` mientras se navega; null = escribiendo algo nuevo
let draft = "";

function remember(text) {
  if (text.startsWith("/key ") || sent[sent.length - 1] === text) return;
  sent.push(text);
  if (sent.length > 50) sent.shift();
  try {
    localStorage.setItem("zen-sent", JSON.stringify(sent));
  } catch {
    // sin almacenamiento disponible: el historial solo dura mientras el chat esté abierto
  }
}

input.addEventListener("keydown", (e) => {
  if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
  if (!palette.hidden) return; // con la paleta abierta, las flechas eligen comando
  if (e.shiftKey || e.altKey || e.ctrlKey || e.metaKey) return;
  if (e.key === "ArrowUp") {
    if (!sent.length || recallIndex === 0) return;
    if (recallIndex === null) {
      draft = input.value;
      recallIndex = sent.length;
    }
    recallIndex -= 1;
  } else {
    if (recallIndex === null) return;
    recallIndex += 1;
  }
  e.preventDefault();
  if (recallIndex >= sent.length) {
    recallIndex = null;
    input.value = draft;
  } else {
    input.value = sent[recallIndex];
  }
  input.setSelectionRange(input.value.length, input.value.length);
});
input.addEventListener("input", () => (recallIndex = null));

form.addEventListener("submit", (e) => {
  e.preventDefault();
  const text = input.value.trim();
  const attached = pendingImages.splice(0);
  if (!text && !attached.length) return;
  attachmentsEl.querySelector(".attach-note")?.remove();
  recallIndex = null;
  input.value = "";
  hidePalette();
  closeLook();
  renderAttachments();
  // «/modelo» no se envía a la IA: abre el selector de modelo de la cabecera.
  if (text.toLowerCase() === "/modelo" && !attached.length) return openModelMenu();
  if (text) remember(text);
  hideIdeas();
  // La API key no debe quedar a la vista en el historial.
  const shown = text.startsWith("/key ") ? "/key ••••••••" : text || "Mira esta imagen y dime qué ves.";
  addUserMessage(shown, attached);
  invoke("submit_message", { text, images: attached.map(({ mime, data }) => ({ mime, data })) });
});

document.getElementById("close").addEventListener("click", () => invoke("hide_chat"));
window.addEventListener("keydown", (e) => {
  // Escape cierra primero lo que esté abierto (paleta, selector de modelo) y solo después la ventana.
  if (e.key === "Escape" && !closeOverlays()) invoke("hide_chat");
});
appWindow.onFocusChanged(({ payload: focused }) => {
  if (focused) input.focus();
});

// El nombre del asistente lo elige el usuario (Ajustes) y se muestra en la cabecera y en el saludo.
const titleEl = document.getElementById("assistant-title");
let assistantName = "Ryan";

function applyName(name) {
  assistantName = name;
  titleEl.textContent = name;
  input.placeholder = `Cuéntale a ${name} qué necesitas…`;
}

// Un saludo que cambia con la hora, como el de un amigo y no el de un servicio.
function greeting() {
  const hour = new Date().getHours();
  const hello = hour >= 5 && hour < 12 ? "¡Buenos días!" : hour >= 12 && hour < 20 ? "¡Hola!" : "¡Buenas noches!";
  const closing = hour >= 5 && hour < 12 ? "¿Empezamos el día juntos?" : hour >= 12 && hour < 20 ? "¿Qué hacemos hoy?" : "¿Te acompaño un rato?";
  return `${hello} Soy ${assistantName}, tu compañero de escritorio 💛 ${closing}`;
}

// Ideas para arrancar: al pulsar una se escribe en la caja (sin enviarla) para que la ajustes a tu gusto.
const IDEAS = [
  ["🎵", "Pon música tranquila"],
  ["⏰", "Recuérdame descansar en 30 minutos"],
  ["📂", "Organiza mi carpeta de Descargas"],
  ["🔎", "Busca algo interesante para hoy"],
];
let ideasEl = null;

function showIdeas() {
  ideasEl = document.createElement("div");
  ideasEl.className = "ideas";
  for (const [icon, text] of IDEAS) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "idea";
    button.textContent = `${icon} ${text}`;
    button.addEventListener("click", () => {
      input.value = text;
      input.focus();
      input.setSelectionRange(text.length, text.length);
    });
    ideasEl.append(button);
  }
  addToHistory(ideasEl);
}

function hideIdeas() {
  ideasEl?.remove();
  ideasEl = null;
}

invoke("get_assistant_name")
  .then(applyName)
  .catch(() => {})
  .finally(() => {
    addMessage("assistant", greeting());
    showIdeas();
  });
window.__TAURI__.event.listen("assistant-name-changed", (e) => applyName(e.payload));
const STATUS_TEXT = {
  idle: "Aquí contigo", walking: "Estirando las patitas", sleeping: "Zzz… soñando", thinking: "Pensando…", working: "Manos a la obra…",
  success: "¡Listo, lo logramos!", happy: "¡Qué alegría!", surprised: "¡Ay! Algo falló", interacting: "¡Hola, hola!", alert: "¡Ojo, aviso!",
};
// Cómo luce la cabecera según el ánimo: ocupado, adormilado o tranquilo.
const MOOD_OF = { thinking: "busy", working: "busy", sleeping: "sleepy" };
window.__TAURI__.event.listen("character-state", (e) => {
  statusEl.textContent = STATUS_TEXT[e.payload] ?? e.payload;
  const state = e.payload;
  document.body.dataset.mood = MOOD_OF[state] ?? "calm";
  playFriend(state);
  // Entre dos pasos de una misma tarea no se aparenta haber terminado.
  const midTask = state === "success" && taskProgress && taskProgress.step < taskProgress.total;
  if (!midTask && state !== "thinking" && state !== "working" && state !== "success") taskProgress = null;
  setBusy(midTask ? "working" : state === "thinking" || state === "working" ? state : null);
});
window.__TAURI__.event.listen("chat-log", (e) => addMessage("assistant", e.payload));

// Respuesta en directo: el texto va apareciendo en una burbuja y, al terminar, esta se sustituye por el mensaje
// completo (ya con formato). Si se interrumpe, lo escrito se queda marcado como incompleto.
function streamDelta(delta) {
  const nearBottom = history.scrollHeight - history.scrollTop - history.clientHeight < 80;
  if (!streamEl) {
    streamEl = document.createElement("div");
    streamEl.className = "msg assistant streaming";
    streamText = "";
    renderTyping();
    addToHistory(streamEl);
  }
  streamText += delta;
  streamEl.textContent = streamText;
  if (nearBottom) history.scrollTop = history.scrollHeight;
}

// Cierra la burbuja en directo. Devuelve false si no había ninguna abierta.
function endStream(finalText) {
  const el = streamEl;
  streamEl = null;
  streamText = "";
  if (!el) return false;
  if (finalText) {
    el.replaceWith(buildMessage("assistant", finalText));
  } else if (el.textContent.trim()) {
    el.classList.replace("streaming", "interrupted");
  } else {
    el.remove();
  }
  renderTyping();
  history.scrollTop = history.scrollHeight;
  return true;
}
window.__TAURI__.event.listen("chat-stream", (e) => streamDelta(e.payload));
window.__TAURI__.event.listen("chat-final", (e) => {
  if (!endStream(e.payload)) addMessage("assistant", e.payload);
});
window.__TAURI__.event.listen("chat-stream-abort", () => endStream(null));
// Lo que se pide desde la barra rápida también queda en la conversación del chat.
window.__TAURI__.event.listen("quick-user-message", (e) => {
  hideIdeas();
  addUserMessage(e.payload);
});
window.__TAURI__.event.listen("reminder", (e) => addMessage("assistant reminder", `⏰ ${e.payload}`));
function showConfirmation({ id, tool, preview, risk, detailed }) {
  const card = document.createElement("div");
  card.className = `msg confirm ${risk}`;

  const title = document.createElement("strong");
  title.textContent = detailed ? `⚠ Acción peligrosa: ${tool}` : `¿Permites esta acción? (${tool})`;
  const body = document.createElement("div");
  body.className = "preview";
  body.textContent = preview;

  const actions = document.createElement("div");
  actions.className = "actions";
  const ok = document.createElement("button");
  ok.textContent = detailed ? "Sí, entiendo y quiero continuar" : "Confirmar";
  const cancel = document.createElement("button");
  cancel.textContent = "Cancelar";
  cancel.className = "secondary";

  pendingConfirmations += 1;
  renderTyping();
  const answer = (approve) => {
    ok.disabled = cancel.disabled = true;
    actions.textContent = approve ? "Confirmado" : "Cancelado";
    invoke("resolve_confirmation", { id, approve });
    pendingConfirmations -= 1;
    renderTyping();
    input.focus();
  };
  ok.addEventListener("click", () => answer(true));
  cancel.addEventListener("click", () => answer(false));

  actions.append(ok, cancel);
  card.append(title, body, actions);
  addToHistory(card);
}
window.__TAURI__.event.listen("confirmation-request", (e) => showConfirmation(e.payload));

// Avatar de la cabecera: el personaje elegido (se actualiza al cambiarlo en Ajustes).
const headerAvatar = document.getElementById("header-avatar");

// Milisegundos por frame, como en la mascota; en la cabecera van algo más lentos para no distraer mientras escribes.
const FRAME_MS = { idle: 110, walking: 90, thinking: 130, working: 70, success: 80, happy: 70, surprised: 90, interacting: 80, alert: 70, sleeping: 260 };
const HEADER_SLOWDOWN = 1.7;
let friend = null; // { entry, base, sets }
let friendState = "idle";
let friendFrames = [];
let friendIndex = 0;
let friendTimer = null;

function hexToRgb(hex) {
  const match = /^#([0-9a-f]{6})$/i.exec(hex ?? "");
  if (!match) return null;
  const n = parseInt(match[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

// Cada amigo tiene sus colores (manifiesto): tiñen el chat entero. Sin ellos se queda el violeta de siempre.
function applyTheme(entry) {
  const root = document.documentElement.style;
  const accent = hexToRgb(entry.accent);
  const accent2 = hexToRgb(entry.accent2);
  if (accent) root.setProperty("--accent-rgb", accent.join(" "));
  else root.removeProperty("--accent-rgb");
  if (accent2) root.setProperty("--accent-2-rgb", accent2.join(" "));
  else root.removeProperty("--accent-2-rgb");
  root.setProperty("--friend-img", `url("${previewUrl(entry)}")`);
}

function playFriend(state) {
  friendState = state;
  clearInterval(friendTimer);
  if (!friend) return;
  const own = friend.sets[state];
  friendFrames = own?.length ? own : friend.base;
  friendIndex = 0;
  headerAvatar.src = friendFrames[0].src;
  // Dormido o en reposo basta con un solo cuadro si no hay nada más que mover.
  if (friendFrames.length < 2) return;
  const ms = (FRAME_MS[state] ?? FRAME_MS.idle) * (friend.entry.speed ?? 1) * HEADER_SLOWDOWN;
  friendTimer = setInterval(() => {
    friendIndex = (friendIndex + 1) % friendFrames.length;
    headerAvatar.src = friendFrames[friendIndex].src;
  }, ms);
}

async function loadHeaderAvatar() {
  try {
    characterManifest ??= await fetch("assets/characters.json").then((response) => response.json());
    const id = await invoke("get_character");
    const entry = characterManifest.characters.find((c) => c.id === id) ?? characterManifest.characters[0];
    applyTheme(entry);
    const load = (spec) =>
      Array.from({ length: spec.frames }, (_, i) => {
        const img = new Image();
        img.src = frameUrl(spec, i + 1);
        return img;
      });
    const base = load(entry);
    const sets = {};
    for (const [state, spec] of Object.entries(entry.states ?? {})) {
      sets[state] = Array.isArray(spec.sequence) ? spec.sequence.map((n) => base[n - 1]).filter(Boolean) : load(spec);
    }
    friend = { entry, base, sets };
    headerAvatar.hidden = false;
    playFriend(friendState);
  } catch {
    friend = null;
    clearInterval(friendTimer);
    headerAvatar.hidden = true;
  }
}
window.__TAURI__.event.listen("character-changed", loadHeaderAvatar);

// --- Pestañas ---
const tabs = document.querySelectorAll(".tab");
const views = {
  today: document.getElementById("view-today"),
  history: document.getElementById("view-history"),
  chat: document.getElementById("view-chat"),
  reminders: document.getElementById("view-reminders"),
  settings: document.getElementById("view-settings"),
};

function showTab(name) {
  tabs.forEach((tab) => {
    const active = tab.dataset.tab === name;
    tab.classList.toggle("active", active);
    tab.setAttribute("aria-selected", String(active));
  });
  Object.entries(views).forEach(([key, view]) => (view.hidden = key !== name));
  if (name === "reminders") openReminders();
  else closeReminders();
  if (name === "today") openToday();
  else closeToday();
  if (name === "history") openHistory();
  if (name === "settings") loadSettings();
  else if (name === "chat") input.focus();
}
tabs.forEach((tab) => tab.addEventListener("click", () => showTab(tab.dataset.tab)));

// --- Recordatorios ---
const reminderForm = document.getElementById("reminder-form");
const reminderText = document.getElementById("reminder-text");
const reminderMode = document.getElementById("reminder-mode");
const reminderIn = document.getElementById("reminder-in");
const reminderMinutes = document.getElementById("reminder-minutes");
const reminderClock = document.getElementById("reminder-clock");
const reminderAdd = document.getElementById("reminder-add");
const reminderPresets = document.getElementById("reminder-presets");
const reminderMessage = document.getElementById("reminder-message");
const reminderList = document.getElementById("reminder-list");
const reminderEmpty = document.getElementById("reminder-empty");
const reminderCount = document.getElementById("reminder-count");
const reminderBadge = document.getElementById("reminder-badge");
let reminders = [];
let reminderMax = 50;
let reminderTick = null;

// «en 12 min», «en 2 h 5 min», «en 3 d».
function relativeTime(seconds) {
  if (seconds <= 0) return "ahora";
  if (seconds < 60) return "en menos de 1 min";
  const minutes = Math.ceil(seconds / 60);
  if (minutes < 60) return `en ${minutes} min`;
  const hours = Math.floor(minutes / 60);
  const restMinutes = minutes % 60;
  if (hours < 24) return restMinutes ? `en ${hours} h ${restMinutes} min` : `en ${hours} h`;
  const days = Math.floor(hours / 24);
  const restHours = hours % 24;
  return restHours ? `en ${days} d ${restHours} h` : `en ${days} d`;
}

// «Hoy 19:30», «Mañana 08:00», «Jueves 5 nov 10:15».
function whenLabel(due) {
  const date = new Date(due * 1000);
  const dayStart = (d) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((dayStart(date) - dayStart(new Date())) / 86400000);
  const time = date.toLocaleTimeString("es", { hour: "2-digit", minute: "2-digit" });
  if (days <= 0) return `Hoy ${time}`;
  if (days === 1) return `Mañana ${time}`;
  const day = date.toLocaleDateString("es", { weekday: "long", day: "numeric", month: "short" });
  return `${day.charAt(0).toUpperCase()}${day.slice(1)} ${time}`;
}

function reminderRow(item) {
  const row = document.createElement("div");
  row.className = "reminder";
  row.dataset.due = String(item.due);

  const when = document.createElement("div");
  when.className = "when";
  const label = document.createElement("strong");
  label.textContent = whenLabel(item.due);
  const left = document.createElement("span");
  left.className = "left";
  when.append(label, left);

  const text = document.createElement("div");
  text.className = "text";
  text.textContent = item.text;

  const cancel = document.createElement("button");
  cancel.type = "button";
  cancel.className = "cancel";
  cancel.textContent = "✕";
  cancel.title = cancel.ariaLabel = "Cancelar este recordatorio";
  cancel.addEventListener("click", async () => {
    cancel.disabled = true;
    try {
      await invoke("cancel_reminder", { id: item.id });
      setMessage(reminderMessage, "Recordatorio cancelado.", "ok");
    } catch (error) {
      setMessage(reminderMessage, String(error), "error");
    }
    await loadReminders();
  });

  row.append(when, text, cancel);
  return row;
}

// La cuenta atrás se recalcula cada segundo sin volver a pedir la lista.
function refreshCountdowns(list = reminderList) {
  const now = Date.now() / 1000;
  for (const row of list.children) {
    const seconds = Number(row.dataset.due) - now;
    row.querySelector(".left").textContent = relativeTime(seconds);
    row.classList.toggle("soon", seconds <= 300);
  }
}

function renderReminders() {
  reminderList.replaceChildren(...reminders.map(reminderRow));
  const count = reminders.length;
  reminderEmpty.hidden = count > 0;
  reminderCount.textContent = count ? `${count}/${reminderMax}` : "";
  reminderBadge.textContent = String(count);
  reminderBadge.hidden = count === 0;
  refreshCountdowns();
  renderTodayReminders();
}

async function loadReminders() {
  try {
    const data = await invoke("list_reminders");
    reminders = data.items;
    reminderMax = data.max;
    renderReminders();
  } catch {
    // sin la lista no hay nada que pintar; se reintenta en el siguiente ciclo
  }
}

function openReminders() {
  loadReminders();
  clearInterval(reminderTick);
  let ticks = 0;
  reminderTick = setInterval(() => {
    refreshCountdowns();
    // Cada pocos segundos se vuelve a leer: así aparecen los que crea la IA y desaparecen los que ya sonaron.
    if (++ticks % 5 === 0) loadReminders();
  }, 1000);
}

function closeReminders() {
  clearInterval(reminderTick);
  reminderTick = null;
}

function showReminderMode() {
  const at = reminderMode.value === "at";
  reminderIn.hidden = at;
  reminderClock.hidden = !at;
  reminderPresets.hidden = at;
}
reminderMode.addEventListener("change", showReminderMode);

reminderPresets.addEventListener("click", (e) => {
  const preset = e.target.closest("[data-minutes]");
  if (!preset) return;
  reminderMinutes.value = preset.dataset.minutes;
  reminderText.focus();
});

reminderForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  const text = reminderText.value.trim();
  if (!text) return setMessage(reminderMessage, "Escribe qué quieres que te recuerde.", "error");
  const byClock = reminderMode.value === "at";
  const minutes = Number(reminderMinutes.value);
  if (byClock ? !reminderClock.value : !Number.isInteger(minutes) || minutes < 1) {
    return setMessage(reminderMessage, byClock ? "Elige la hora." : "Indica los minutos (un número entero).", "error");
  }
  reminderAdd.disabled = true;
  try {
    const when = await invoke("add_reminder", {
      text,
      minutes: byClock ? null : minutes,
      at: byClock ? reminderClock.value : null,
    });
    reminderText.value = "";
    setMessage(reminderMessage, `Listo, te lo recordaré ${when}.`, "ok");
  } catch (error) {
    setMessage(reminderMessage, String(error), "error");
  } finally {
    reminderAdd.disabled = false;
    await loadReminders();
  }
});

// Cuando suena uno, o cuando la IA termina una tarea (quizá creó uno), se actualiza la lista y el contador.
window.__TAURI__.event.listen("reminder", loadReminders);
window.__TAURI__.event.listen("character-state", (e) => {
  if (e.payload === "success") loadReminders();
});
loadReminders();

// Secciones de Ajustes: una a la vez, para no tener que recorrer una lista larguísima.
const subtabs = [...document.querySelectorAll(".subtab")];
const panels = [...document.querySelectorAll(".panel")];

function showPanel(name) {
  subtabs.forEach((tab) => {
    const active = tab.dataset.panel === name;
    tab.classList.toggle("active", active);
    tab.setAttribute("aria-selected", String(active));
  });
  panels.forEach((panel) => (panel.hidden = panel.dataset.panel !== name));
  document.querySelector(".panels").scrollTop = 0;
}
subtabs.forEach((tab) => tab.addEventListener("click", () => showPanel(tab.dataset.panel)));

// --- Ajustes ---
const providerSelect = document.getElementById("provider");
const modelForm = document.getElementById("model-form");
const modelInput = document.getElementById("model");
const modelHint = document.getElementById("model-hint");
const modelMessage = document.getElementById("model-message");
const saveModel = document.getElementById("save-model");
const keyForm = document.getElementById("key-form");
const keyLabel = document.getElementById("key-label");
const keyInput = document.getElementById("api-key");
const keyStatus = document.getElementById("key-status");
const keyHint = document.getElementById("key-hint");
const keyMessage = document.getElementById("key-message");
const saveKey = document.getElementById("save-key");
const deleteKey = document.getElementById("delete-key");
const ollamaForm = document.getElementById("ollama-form");
const ollamaUrl = document.getElementById("ollama-url");
const ollamaMessage = document.getElementById("ollama-message");
const saveOllama = document.getElementById("save-ollama");
const detectModels = document.getElementById("detect-models");
const modelOptions = document.getElementById("model-options");

const shortcutForm = document.getElementById("shortcut-form");
const shortcutKey = document.getElementById("shortcut-key");
const shortcutMessage = document.getElementById("shortcut-message");
const shortcutHint = document.getElementById("shortcut-hint");
const saveShortcut = document.getElementById("save-shortcut");
const modBoxes = [...document.querySelectorAll("[data-mod]")];
const autostartBox = document.getElementById("autostart");
const autostartMessage = document.getElementById("autostart-message");
const wanderBox = document.getElementById("behavior-wander");
const lookBox = document.getElementById("behavior-look");
const musicBox = document.getElementById("music-reaction");
const behaviorMessage = document.getElementById("behavior-message");
const soundBox = document.getElementById("sound-enabled");
const volumeInput = document.getElementById("sound-volume");
const volumeValue = document.getElementById("sound-volume-value");
const testSoundButton = document.getElementById("test-sound");
const batteryBox = document.getElementById("battery-alerts");
const systemEventsBox = document.getElementById("system-events");
const soundMessage = document.getElementById("sound-message");
const usageBox = document.getElementById("usage-comments");
const memoryBox = document.getElementById("memory-enabled");
const memoryDetails = document.getElementById("memory-details");
const memoryList = document.getElementById("memory-list");
const memoryMessage = document.getElementById("memory-message");
const ccState = document.getElementById("cc-state");
const ccToggle = document.getElementById("cc-toggle");
const ccWarning = document.getElementById("cc-warning");
const ccMessage = document.getElementById("cc-message");
const confirmAllBox = document.getElementById("confirm-all");
const toolsList = document.getElementById("tools-list");
const toolsSearch = document.getElementById("tools-search");
const toolsCount = document.getElementById("tools-count");
const toolsAllOn = document.getElementById("tools-all-on");
const toolsAllOff = document.getElementById("tools-all-off");
let toolsVisible = [];
const toolsMessage = document.getElementById("tools-message");
const avatarsEl = document.getElementById("avatars");
const avatarMessage = document.getElementById("avatar-message");

let settings = null;

const currentProvider = () => settings.providers.find((p) => p.id === settings.provider);

function setMessage(el, text, kind = "") {
  el.textContent = text;
  el.className = `message ${kind}`;
}

// Las claves nunca vuelven al frontend: solo se sabe si existen y de dónde vienen.
function renderSettings() {
  const p = currentProvider();
  if (providerSelect.options.length !== settings.providers.length) {
    providerSelect.replaceChildren(
      ...settings.providers.map((item) => new Option(item.name, item.id)),
    );
  }
  providerSelect.value = p.id;

  modelInput.value = p.model;
  modelInput.placeholder = p.default_model;
  modelHint.textContent = `Si lo dejas vacío se usa ${p.default_model}.`;

  // Un modelo local no tiene clave: se muestra la dirección del servidor en su lugar.
  keyForm.hidden = !p.needs_key;
  ollamaForm.hidden = p.needs_key;
  if (!p.needs_key && document.activeElement !== ollamaUrl) ollamaUrl.value = settings.ollama_url;

  keyLabel.textContent = `API key de ${p.name}`;
  const sources = {
    stored: "Hay una clave guardada. Escribe una nueva para reemplazarla.",
    env: `Usando la variable de entorno ${p.env_var}. Guarda una clave para que tenga prioridad.`,
    none: "Todavía no hay ninguna clave.",
  };
  keyStatus.textContent = sources[p.key_source] ?? "";
  keyInput.placeholder = p.key_source === "stored" ? "•••••••••••• (guardada)" : "Pega tu API key";
  keyHint.textContent = `Se guarda en el Administrador de credenciales de Windows, no en un archivo. Consíguela en ${p.key_url}.`;
  deleteKey.disabled = p.key_source !== "stored";

  renderShortcut();
  renderQuickShortcut();
  autostartBox.checked = settings.autostart;
  renderAvatars();
  nameInput.value = settings.assistant_name;
  wanderBox.checked = settings.wander;
  lookBox.checked = settings.look_at_cursor;
  musicBox.checked = settings.music_reaction;
  soundBox.checked = settings.sound_enabled;
  volumeInput.value = settings.sound_volume;
  volumeInput.disabled = testSoundButton.disabled = !settings.sound_enabled;
  volumeValue.textContent = `${settings.sound_volume}%`;
  batteryBox.checked = settings.battery_alerts;
  systemEventsBox.checked = settings.system_events;
  usageBox.checked = settings.usage_comments;
  memoryBox.checked = settings.memory_enabled;
  ccState.textContent = settings.computer_control ? "activado" : "desactivado";
  ccToggle.textContent = settings.computer_control ? "Desactivar" : "Activar…";
  if (settings.computer_control) ccWarning.hidden = true;
  confirmAllBox.checked = settings.confirm_all;
  renderTools();
  ollamaThinkBox.checked = Boolean(settings.ollama_think);
  historySave.checked = settings.save_history !== false;
  updateModelPill();
  renderProfile();
}

// --- Aplicaciones que la IA puede abrir ---
const appsDetails = document.getElementById("apps-details");
const appsLoading = document.getElementById("apps-loading");
const appsBuiltin = document.getElementById("apps-builtin");
const appsList = document.getElementById("apps-list");
const appsFilter = document.getElementById("apps-filter");
const appsRefresh = document.getElementById("apps-refresh");
const appsHint = document.getElementById("apps-hint");
const appsMessage = document.getElementById("apps-message");
const appsOnlyEnabled = document.getElementById("apps-only-enabled");
const appsAllOn = document.getElementById("apps-all-on");
const appsAllOff = document.getElementById("apps-all-off");
const appsCount = document.getElementById("apps-count");
let appsData = null;
let appsShown = [];

function appCheckbox(name, checked, note, onChange) {
  const label = document.createElement("label");
  label.className = "check";
  const box = document.createElement("input");
  box.type = "checkbox";
  box.checked = checked;
  box.addEventListener("change", () => onChange(box));
  const text = document.createElement("span");
  const title = document.createElement("span");
  title.textContent = name;
  text.append(title);
  if (note) {
    const small = document.createElement("span");
    small.className = "note";
    small.textContent = ` · ${note}`;
    text.append(small);
  }
  label.append(box, text);
  return label;
}

async function changeApp(command, args) {
  try {
    await invoke(command, args);
    setMessage(appsMessage, "");
  } catch (error) {
    setMessage(appsMessage, String(error), "error");
  }
  await loadApps(false);
}

function renderApps() {
  if (!appsData) return;
  appsBuiltin.replaceChildren(
    ...appsData.builtin.map((app) =>
      appCheckbox(app.name, app.enabled, app.installed ? "" : "no está instalada", (box) =>
        changeApp("set_builtin_app", { key: app.key, enabled: box.checked }),
      ),
    ),
  );
  const needle = appsFilter.value.trim().toLowerCase();
  const shown = appsData.apps.filter((app) => app.name.toLowerCase().includes(needle) && (!appsOnlyEnabled.checked || app.enabled));
  const enabledCount = appsData.apps.filter((app) => app.enabled).length;
  appsCount.textContent = `${enabledCount} de ${appsData.apps.length} activadas${shown.length !== appsData.apps.length ? ` · se ven ${shown.length}` : ""}.`;
  appsShown = shown;
  appsList.replaceChildren(
    ...(shown.length
      ? shown.map((app) => appCheckbox(app.name, app.enabled, "", (box) => changeApp("set_custom_app", { app_id: app.app_id, enabled: box.checked })))
      : [Object.assign(document.createElement("span"), { className: "hint", textContent: needle ? "Ninguna coincide." : "No se encontraron aplicaciones." })]),
  );
  appsHint.textContent = appsData.hidden_tools
    ? `Por seguridad no se ofrecen ${appsData.hidden_tools} herramientas del sistema (consola, PowerShell, terminal, registro…).`
    : "";
  if (appsData.error) setMessage(appsMessage, appsData.error, "error");
}

async function loadApps(refresh) {
  appsLoading.hidden = false;
  appsRefresh.disabled = true;
  try {
    appsData = await invoke("list_apps", { refresh });
  } catch (error) {
    setMessage(appsMessage, String(error), "error");
  } finally {
    appsLoading.hidden = true;
    appsRefresh.disabled = false;
  }
  renderApps();
}

appsDetails.addEventListener("toggle", () => {
  if (appsDetails.open && !appsData) loadApps(false);
});
appsRefresh.addEventListener("click", () => loadApps(true));
appsFilter.addEventListener("input", renderApps);
appsOnlyEnabled.addEventListener("change", renderApps);

// Activar o desactivar de golpe las aplicaciones que se ven con el filtro actual.
async function bulkApps(enabled) {
  const ids = appsShown.filter((app) => app.enabled !== enabled).map((app) => app.app_id);
  if (!ids.length) return setMessage(appsMessage, enabled ? "Ya están todas activadas." : "Ya están todas desactivadas.", "ok");
  try {
    const warning = await invoke("set_custom_apps", { appIds: ids, enabled });
    setMessage(appsMessage, warning || `${ids.length} aplicaciones ${enabled ? "activadas" : "desactivadas"}.`, warning ? "error" : "ok");
  } catch (error) {
    setMessage(appsMessage, String(error), "error");
  }
  await loadApps(false);
}
appsAllOn.addEventListener("click", () => bulkApps(true));
appsAllOff.addEventListener("click", () => bulkApps(false));

// Los personajes se describen en assets/characters.json, el mismo archivo que usa la mascota.
let characterManifest = null;

function frameUrl(entry, n) {
  const name = entry.pattern.replace("{nn}", String(n).padStart(2, "0")).replace("{n}", String(n));
  return `assets/${entry.folder}/${name}`;
}

function previewUrl(entry) {
  return frameUrl(entry, entry.preview_frame);
}

async function renderAvatars() {
  characterManifest ??= await fetch("assets/characters.json").then((response) => response.json());
  // Las tarjetas se crean una sola vez; después solo se actualiza cuál está marcada.
  if (avatarsEl.children.length !== characterManifest.characters.length) {
    avatarsEl.replaceChildren(
      ...characterManifest.characters.map((entry) => {
        const label = document.createElement("label");
        label.className = "avatar";
        const radio = document.createElement("input");
        radio.type = "radio";
        radio.name = "avatar";
        radio.value = entry.id;
        radio.addEventListener("change", () => changeSetting(avatarMessage, "set_character", { id: entry.id }));
        const image = document.createElement("img");
        image.alt = "";
        image.src = previewUrl(entry);
        const name = document.createElement("span");
        name.textContent = entry.name;
        label.append(radio, image, name);
        return label;
      }),
    );
  }
  avatarsEl.querySelectorAll("input").forEach((radio) => (radio.checked = radio.value === settings.character));
}

// Atajo guardado -> casillas de modificadores + tecla.
function renderShortcut() {
  const parts = settings.shortcut.split("+");
  shortcutKey.value = parts[parts.length - 1];
  modBoxes.forEach((box) => (box.checked = parts.slice(0, -1).includes(box.dataset.mod)));
  const label = (text) => text.replace("Super", "Win");
  const active = settings.shortcut_active;
  shortcutHint.textContent =
    active === null
      ? "No se pudo registrar ningún atajo: elige otro."
      : active !== settings.shortcut
        ? `${label(settings.shortcut)} no está disponible; ahora se usa ${label(active)}.`
        : `Ahora: ${label(active)}.`;
}

// Las herramientas se agrupan por categoría y cada una dice cuándo te pide permiso.
const TOOL_CATEGORIES = [
  ["Archivos", ["create_file", "create_folder", "list_files", "read_file", "search_files", "move_file", "copy_file", "delete_file", "organize_folder", "undo_organize", "open_path", "zip_files", "unzip_file"]],
  ["Espacio en disco", ["find_large_files", "folder_sizes", "find_duplicates", "empty_recycle_bin"]],
  ["Música y multimedia", ["play_music", "search_music", "media_control"]],
  ["Internet y aplicaciones", ["web_search", "open_website", "open_application"]],
  ["Sistema", ["system_info", "set_brightness", "read_clipboard", "write_clipboard"]],
  ["Recordatorios y memoria", ["set_reminder", "list_reminders", "cancel_reminder", "remember", "forget"]],
  ["Pantalla y otras ventanas", ["look_at_screen", "list_windows", "list_controls", "type_text", "press_keys", "click_control"]],
];
// Estas solo funcionan si además está activado «Control de otras aplicaciones».
const CONTROL_TOOLS = ["list_windows", "list_controls", "type_text", "press_keys", "click_control"];
const RISK_LABEL = { safe: "Segura", confirm: "Pide permiso", dangerous: "Peligrosa" };

// Qué grupos dejó desplegados el usuario (se conserva al volver a dibujar la lista).
const openGroups = new Set();

const categoryOf = (name) => TOOL_CATEGORIES.find(([, names]) => names.includes(name))?.[0] ?? "Otras";
const humanToolName = (name) => name.replaceAll("_", " ");

function toolRow(tool) {
  const row = document.createElement("label");
  row.className = "tool-row";
  const box = document.createElement("input");
  box.type = "checkbox";
  box.checked = tool.enabled;
  box.setAttribute("aria-label", `Permitir ${humanToolName(tool.name)}`);
  box.addEventListener("change", () => changeSetting(toolsMessage, "set_tool_enabled", { name: tool.name, enabled: box.checked }));

  const body = document.createElement("div");
  body.className = "tool-body";
  const head = document.createElement("div");
  head.className = "tool-head";
  const name = document.createElement("strong");
  name.textContent = humanToolName(tool.name);
  const risk = document.createElement("span");
  risk.className = `risk ${tool.risk}`;
  risk.textContent = RISK_LABEL[tool.risk] ?? tool.risk;
  head.append(name, risk);
  if (CONTROL_TOOLS.includes(tool.name) && !settings.computer_control) {
    const note = document.createElement("span");
    note.className = "risk muted";
    note.textContent = "necesita el control de otras aplicaciones";
    head.append(note);
  }
  const desc = document.createElement("p");
  desc.className = "desc";
  desc.textContent = tool.description;
  body.append(head, desc);

  row.append(box, body);
  return row;
}

function toolGroup(category, tools, forceOpen) {
  const details = document.createElement("details");
  details.className = "tool-group";
  details.open = forceOpen || openGroups.has(category);
  details.addEventListener("toggle", () => (details.open ? openGroups.add(category) : openGroups.delete(category)));

  const summary = document.createElement("summary");
  const name = document.createElement("span");
  name.className = "g-name";
  name.textContent = category;
  const count = document.createElement("span");
  count.className = "g-count";
  count.textContent = `${tools.filter((t) => t.enabled).length}/${tools.length}`;
  const all = tools.every((t) => t.enabled);
  const toggle = document.createElement("button");
  toggle.type = "button";
  toggle.className = "g-toggle";
  toggle.textContent = all ? "Ninguna" : "Todas";
  toggle.title = all ? "Desactivar todas las de este grupo" : "Activar todas las de este grupo";
  // El botón vive en la cabecera del acordeón: pulsarlo no debe plegarlo ni desplegarlo.
  toggle.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    changeSetting(toolsMessage, "set_tools_enabled", { names: tools.map((t) => t.name), enabled: !all });
  });
  summary.append(name, count, toggle);
  details.append(summary, ...tools.map(toolRow));
  return details;
}

function renderTools() {
  const needle = toolsSearch.value.trim().toLowerCase();
  const all = settings.tools;
  toolsCount.textContent = `${all.filter((t) => t.enabled).length}/${all.length}`;
  const visible = all.filter((t) => !needle || humanToolName(t.name).includes(needle) || t.description.toLowerCase().includes(needle));
  toolsVisible = visible;

  const byCategory = new Map();
  for (const tool of visible) {
    const category = categoryOf(tool.name);
    if (!byCategory.has(category)) byCategory.set(category, []);
    byCategory.get(category).push(tool);
  }
  const order = [...TOOL_CATEGORIES.map(([category]) => category), "Otras"];
  const groups = order.filter((category) => byCategory.has(category)).map((category) => toolGroup(category, byCategory.get(category), Boolean(needle)));
  toolsList.replaceChildren(
    ...(groups.length ? groups : [Object.assign(document.createElement("p"), { className: "hint", textContent: "Ninguna herramienta coincide." })]),
  );
}

toolsSearch.addEventListener("input", () => renderTools());
toolsAllOn.addEventListener("click", () => changeSetting(toolsMessage, "set_tools_enabled", { names: toolsVisible.map((t) => t.name), enabled: true }));
toolsAllOff.addEventListener("click", () => changeSetting(toolsMessage, "set_tools_enabled", { names: toolsVisible.map((t) => t.name), enabled: false }));

// --- Nombre del asistente ---
const nameForm = document.getElementById("name-form");
const nameInput = document.getElementById("assistant-name");
const nameMessage = document.getElementById("name-message");
const saveName = document.getElementById("save-name");

nameForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  saveName.disabled = true;
  try {
    await invoke("set_assistant_name", { name: nameInput.value });
    setMessage(nameMessage, "Nombre guardado.", "ok");
  } catch (error) {
    setMessage(nameMessage, String(error), "error");
  } finally {
    saveName.disabled = false;
    await loadSettings();
  }
});

// Aplica un ajuste y vuelve a leer el estado real (si falla, la casilla vuelve a su valor).
async function changeSetting(messageEl, command, args) {
  try {
    await invoke(command, args);
    setMessage(messageEl, "");
  } catch (error) {
    setMessage(messageEl, String(error), "error");
  }
  await loadSettings();
}

wanderBox.addEventListener("change", () => changeSetting(behaviorMessage, "set_behavior", { name: "wander", enabled: wanderBox.checked }));
lookBox.addEventListener("change", () => changeSetting(behaviorMessage, "set_behavior", { name: "look_at_cursor", enabled: lookBox.checked }));
musicBox.addEventListener("change", () => changeSetting(behaviorMessage, "set_behavior", { name: "music_reaction", enabled: musicBox.checked }));
// Recuerdos: se leen al abrir la sección y tras cada cambio.
async function loadMemories() {
  try {
    const facts = await invoke("list_memories");
    memoryList.replaceChildren();
    if (!facts.length) {
      const empty = document.createElement("p");
      empty.className = "hint";
      empty.textContent = "Todavía no recuerda nada.";
      memoryList.append(empty);
    }
    for (const fact of facts) {
      const row = document.createElement("div");
      row.className = "memory-item";
      const text = document.createElement("span");
      text.textContent = fact.text;
      const remove = document.createElement("button");
      remove.type = "button";
      remove.textContent = "Olvidar";
      remove.setAttribute("aria-label", `Olvidar: ${fact.text}`);
      remove.addEventListener("click", async () => {
        try {
          await invoke("delete_memory", { id: fact.id });
          setMessage(memoryMessage, "");
        } catch (error) {
          setMessage(memoryMessage, String(error), "error");
        }
        loadMemories();
      });
      row.append(text, remove);
      memoryList.append(row);
    }
    document.getElementById("memory-clear").disabled = !facts.length;
  } catch (error) {
    setMessage(memoryMessage, String(error), "error");
  }
}
memoryDetails.addEventListener("toggle", () => memoryDetails.open && loadMemories());
document.getElementById("memory-clear").addEventListener("click", async () => {
  await invoke("clear_memories");
  loadMemories();
});
usageBox.addEventListener("change", () => changeSetting(memoryMessage, "set_usage_comments", { enabled: usageBox.checked }));
memoryBox.addEventListener("change", () => changeSetting(memoryMessage, "set_memory_enabled", { enabled: memoryBox.checked }));
ccToggle.addEventListener("click", () => {
  if (settings.computer_control) changeSetting(ccMessage, "set_computer_control", { enabled: false });
  else ccWarning.hidden = !ccWarning.hidden;
});
document.getElementById("cc-cancel").addEventListener("click", () => (ccWarning.hidden = true));
document.getElementById("cc-confirm").addEventListener("click", () => changeSetting(ccMessage, "set_computer_control", { enabled: true }));
soundBox.addEventListener("change", () => changeSetting(soundMessage, "set_sound", { enabled: soundBox.checked }));
batteryBox.addEventListener("change", () => changeSetting(soundMessage, "set_sound", { batteryAlerts: batteryBox.checked }));
systemEventsBox.addEventListener("change", () => changeSetting(soundMessage, "set_sound", { systemEvents: systemEventsBox.checked }));
volumeInput.addEventListener("input", () => (volumeValue.textContent = `${volumeInput.value}%`));
volumeInput.addEventListener("change", () => changeSetting(soundMessage, "set_sound", { volume: Number(volumeInput.value) }));
testSoundButton.addEventListener("click", () => invoke("test_sound", { volume: Number(volumeInput.value) }));
confirmAllBox.addEventListener("change", () => changeSetting(toolsMessage, "set_confirm_all", { value: confirmAllBox.checked }));
autostartBox.addEventListener("change", () => changeSetting(autostartMessage, "set_autostart", { enabled: autostartBox.checked }));

// Atajo de la barra rápida: mismo formulario que el del chat.
const quickForm = document.getElementById("quick-form");
const quickKey = document.getElementById("quick-key");
const quickMessage = document.getElementById("quick-message");
const quickHint = document.getElementById("quick-hint");
const saveQuick = document.getElementById("save-quick");
const quickMods = [...document.querySelectorAll("[data-qmod]")];

function renderQuickShortcut() {
  const parts = settings.quick_shortcut.split("+");
  quickKey.value = parts[parts.length - 1];
  quickMods.forEach((box) => (box.checked = parts.slice(0, -1).includes(box.dataset.qmod)));
  const label = (text) => text.replace("Super", "Win");
  quickHint.textContent = settings.quick_shortcut_active
    ? `Ahora: ${label(settings.quick_shortcut_active)}.`
    : `${label(settings.quick_shortcut)} no está disponible (otro programa lo usa): elige otro.`;
}

quickForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  const mods = quickMods.filter((box) => box.checked).map((box) => box.dataset.qmod);
  const shortcut = [...mods, quickKey.value.trim().toUpperCase()].join("+");
  saveQuick.disabled = true;
  try {
    await invoke("set_quick_shortcut", { shortcut });
    setMessage(quickMessage, "Atajo guardado.", "ok");
  } catch (error) {
    setMessage(quickMessage, String(error), "error");
  } finally {
    saveQuick.disabled = false;
    await loadSettings();
  }
});

shortcutForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  const mods = modBoxes.filter((box) => box.checked).map((box) => box.dataset.mod);
  const shortcut = [...mods, shortcutKey.value.trim().toUpperCase()].join("+");
  saveShortcut.disabled = true;
  try {
    await invoke("set_shortcut", { shortcut });
    setMessage(shortcutMessage, "Atajo guardado.", "ok");
  } catch (error) {
    setMessage(shortcutMessage, String(error), "error");
  } finally {
    saveShortcut.disabled = false;
    await loadSettings();
  }
});

async function loadSettings() {
  settings = await invoke("get_settings");
  renderSettings();
}

providerSelect.addEventListener("change", async () => {
  try {
    await invoke("set_provider", { id: providerSelect.value });
    modelOptions.replaceChildren();
    setMessage(ollamaMessage, "");
    setMessage(keyMessage, "");
    setMessage(modelMessage, "");
    await loadSettings();
    addMessage("assistant", `Ahora uso ${currentProvider().name}. Empezamos una conversación nueva.`);
  } catch (error) {
    setMessage(modelMessage, String(error), "error");
    await loadSettings();
  }
});

modelForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  saveModel.disabled = true;
  try {
    await invoke("set_model", { provider: settings.provider, model: modelInput.value });
    setMessage(modelMessage, "Modelo guardado.", "ok");
  } catch (error) {
    setMessage(modelMessage, String(error), "error");
  } finally {
    saveModel.disabled = false;
    await loadSettings();
  }
});

keyForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  const key = keyInput.value.trim();
  if (!key) return setMessage(keyMessage, "Escribe tu API key primero.", "error");
  saveKey.disabled = true;
  try {
    await invoke("save_api_key", { provider: settings.provider, key });
    keyInput.value = "";
    setMessage(keyMessage, "Clave guardada.", "ok");
  } catch (error) {
    setMessage(keyMessage, String(error), "error");
  } finally {
    saveKey.disabled = false;
    await loadSettings();
  }
});

ollamaForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  saveOllama.disabled = true;
  try {
    ollamaUrl.value = await invoke("set_ollama_url", { url: ollamaUrl.value });
    setMessage(ollamaMessage, "Dirección guardada.", "ok");
  } catch (error) {
    setMessage(ollamaMessage, String(error), "error");
  } finally {
    saveOllama.disabled = false;
    await loadSettings();
  }
});

// Sirve de prueba de conexión y rellena las sugerencias del campo «Modelo».
detectModels.addEventListener("click", async () => {
  detectModels.disabled = true;
  setMessage(ollamaMessage, "Buscando…");
  try {
    const models = await invoke("list_ollama_models", { url: ollamaUrl.value });
    modelOptions.replaceChildren(...models.map((name) => new Option(name, name)));
    setMessage(
      ollamaMessage,
      models.length
        ? `Ollama responde. Modelos descargados: ${models.join(", ")}. Elige uno en «Modelo».`
        : "Ollama responde, pero no hay modelos descargados. Prueba: ollama pull qwen3:8b",
      models.length ? "ok" : "error",
    );
  } catch (error) {
    setMessage(ollamaMessage, String(error), "error");
  } finally {
    detectModels.disabled = false;
  }
});

deleteKey.addEventListener("click", async () => {
  try {
    await invoke("delete_api_key", { provider: settings.provider });
    setMessage(keyMessage, "Clave borrada.", "ok");
  } catch (error) {
    setMessage(keyMessage, String(error), "error");
  }
  await loadSettings();
});
// --- Modelo de IA en la cabecera ---
const modelPill = document.getElementById("model-pill");
const modelPillText = document.getElementById("model-pill-text");
const modelMenu = document.getElementById("model-menu");
const ollamaThinkBox = document.getElementById("ollama-think");

const modelInUse = (p) => p.model || p.default_model;

// «qwen3.5:9b · local» para un modelo en tu equipo, «Google Gemini · gemini-…» para uno en la nube.
function updateModelPill() {
  const p = settings && currentProvider();
  if (!p) return;
  modelPillText.textContent = p.needs_key ? `${p.name} · ${modelInUse(p)}` : `${modelInUse(p)} · local`;
  modelPill.title = `Modelo en uso: ${modelInUse(p)} (${p.name}). Pulsa para cambiar.`;
  modelPill.hidden = false;
}

function menuTitle(text) {
  const el = document.createElement("div");
  el.className = "menu-title";
  el.textContent = text;
  return el;
}

function menuHint(text) {
  const el = document.createElement("div");
  el.className = "menu-hint";
  el.textContent = text;
  return el;
}

function menuItem(label, { active = false, note = "", onClick }) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "menu-item";
  button.setAttribute("role", "menuitemradio");
  button.setAttribute("aria-checked", String(active));
  const mark = document.createElement("span");
  mark.className = "mark";
  mark.textContent = active ? "✓" : "";
  const name = document.createElement("span");
  name.className = "label";
  name.textContent = label;
  button.append(mark, name);
  if (note) {
    const small = document.createElement("span");
    small.className = "note";
    small.textContent = note;
    button.append(small);
  }
  button.addEventListener("click", onClick);
  return button;
}

function closeModelMenu() {
  modelMenu.hidden = true;
  modelPill.setAttribute("aria-expanded", "false");
}

async function chooseProvider(id) {
  if (id === settings.provider) return;
  try {
    await invoke("set_provider", { id });
    await loadSettings();
    closeModelMenu();
    addMessage("assistant", `Ahora uso ${currentProvider().name}. Empezamos una conversación nueva.`);
  } catch (error) {
    addMessage("assistant", `No pude cambiar de proveedor: ${error}`);
  }
}

async function chooseModel(provider, model) {
  try {
    await invoke("set_model", { provider, model });
    await loadSettings();
    closeModelMenu();
    addMessage("assistant", `Listo, ahora uso el modelo ${model}.`);
  } catch (error) {
    addMessage("assistant", `No pude cambiar de modelo: ${error}`);
  }
}

async function renderModelMenu() {
  const p = currentProvider();
  const entries = [menuTitle("Proveedor")];
  for (const provider of settings.providers) {
    entries.push(
      menuItem(provider.name, {
        active: provider.id === p.id,
        note: provider.needs_key ? "" : "en tu equipo",
        onClick: () => chooseProvider(provider.id),
      }),
    );
  }
  modelMenu.replaceChildren(...entries);

  if (!p.needs_key) {
    const holder = document.createElement("div");
    holder.className = "menu-models";
    holder.append(menuHint("Buscando modelos…"));
    modelMenu.append(menuTitle("Modelos de Ollama"), holder);

    const think = document.createElement("label");
    think.className = "switch menu-switch";
    const box = document.createElement("input");
    box.type = "checkbox";
    box.checked = Boolean(settings.ollama_think);
    const track = document.createElement("span");
    track.className = "track";
    const text = document.createElement("span");
    text.className = "text";
    text.textContent = "Razonar antes de responder (más lento)";
    think.append(box, track, text);
    box.addEventListener("change", async () => {
      await invoke("set_ollama_think", { value: box.checked }).catch(() => {});
      await loadSettings();
    });
    modelMenu.append(think);

    try {
      const names = await invoke("list_ollama_models", {});
      holder.replaceChildren(
        ...(names.length
          ? names.map((name) => menuItem(name, { active: name === modelInUse(p), onClick: () => chooseModel(p.id, name) }))
          : [menuHint("No hay modelos descargados. Prueba: ollama pull qwen3:8b")]),
      );
    } catch (error) {
      holder.replaceChildren(menuHint(String(error)));
    }
  }

  modelMenu.append(
    menuItem("Más opciones de IA…", {
      onClick: () => {
        closeModelMenu();
        showTab("settings");
        showPanel("ai");
      },
    }),
  );
}

async function openModelMenu() {
  showTab("chat");
  if (!settings) await loadSettings().catch(() => {});
  if (!settings) return;
  modelMenu.hidden = false;
  modelPill.setAttribute("aria-expanded", "true");
  renderModelMenu();
}

modelPill.addEventListener("click", (e) => {
  e.stopPropagation();
  if (modelMenu.hidden) openModelMenu();
  else closeModelMenu();
});
document.addEventListener("click", (e) => {
  if (!modelMenu.hidden && !modelMenu.contains(e.target)) closeModelMenu();
});

// El mismo interruptor que en el menú, dentro de Ajustes > IA.
ollamaThinkBox.addEventListener("change", async () => {
  try {
    await invoke("set_ollama_think", { value: ollamaThinkBox.checked });
    setMessage(ollamaMessage, ollamaThinkBox.checked ? "El modelo razonará antes de responder (más lento)." : "El modelo responderá directamente.", "ok");
  } catch (error) {
    setMessage(ollamaMessage, String(error), "error");
  }
  await loadSettings();
});

// --- Paleta de comandos: al escribir «/» aparece la lista con lo que se puede usar ---
const palette = document.getElementById("palette");
const COMMANDS = [
  { cmd: "/new", desc: "Empezar una conversación nueva" },
  { cmd: "/modelo", desc: "Cambiar de proveedor o de modelo de IA" },
  { cmd: "/tools", desc: "Ver las herramientas disponibles" },
  { cmd: "/key", args: "TU_CLAVE", desc: "Guardar la API key del proveedor activo" },
  { cmd: "/forget", desc: "Borrar la API key guardada" },
];
let paletteItems = [];
let paletteIndex = 0;

function hidePalette() {
  palette.hidden = true;
  paletteItems = [];
}

// Cierra lo que esté abierto encima del chat; devuelve si había algo.
function closeOverlays() {
  const open = !palette.hidden || !modelMenu.hidden || !lookPanel.hidden;
  hidePalette();
  closeModelMenu();
  closeLook();
  return open;
}

function renderPalette() {
  const typed = input.value.toLowerCase();
  // Solo mientras se escribe el comando: en cuanto hay un espacio, empiezan los argumentos.
  paletteItems = typed.startsWith("/") && !/\s/.test(typed) ? COMMANDS.filter((c) => c.cmd.startsWith(typed)) : [];
  if (!paletteItems.length) return hidePalette();
  paletteIndex = Math.min(paletteIndex, paletteItems.length - 1);
  palette.replaceChildren(
    ...paletteItems.map((item, index) => {
      const row = document.createElement("button");
      row.type = "button";
      row.className = "palette-item";
      row.setAttribute("role", "option");
      row.setAttribute("aria-selected", String(index === paletteIndex));
      const cmd = document.createElement("strong");
      cmd.textContent = item.args ? `${item.cmd} ${item.args}` : item.cmd;
      const desc = document.createElement("span");
      desc.textContent = item.desc;
      row.append(cmd, desc);
      // mousedown (y no click) para que la caja de texto no pierda el foco.
      row.addEventListener("mousedown", (e) => {
        e.preventDefault();
        pickCommand(index);
      });
      return row;
    }),
  );
  palette.hidden = false;
}

function pickCommand(index) {
  const item = paletteItems[index];
  if (!item) return;
  input.value = item.args ? `${item.cmd} ` : item.cmd;
  input.focus();
  input.setSelectionRange(input.value.length, input.value.length);
  renderPalette();
}

input.addEventListener("input", () => {
  paletteIndex = 0;
  renderPalette();
});
input.addEventListener("keydown", (e) => {
  if (palette.hidden) return;
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    paletteIndex = (paletteIndex + (e.key === "ArrowDown" ? 1 : -1) + paletteItems.length) % paletteItems.length;
    renderPalette();
  } else if (e.key === "Tab") {
    e.preventDefault();
    pickCommand(paletteIndex);
  } else if (e.key === "Enter" && input.value.trim().toLowerCase() !== paletteItems[paletteIndex].cmd) {
    // Con un comando a medias, Enter lo completa; con el comando ya entero, lo envía.
    e.preventDefault();
    pickCommand(paletteIndex);
  }
});
input.addEventListener("blur", () => setTimeout(hidePalette, 120));

// --- Imágenes adjuntas: pegar capturas y mirar una ventana ---
const attachmentsEl = document.getElementById("attachments");
const lookBtn = document.getElementById("look-btn");
const lookPanel = document.getElementById("look-panel");
const lookStatus = document.getElementById("look-status");
const lookThumb = document.getElementById("look-thumb");
const lookCaption = document.getElementById("look-caption");
const lookSelect = document.getElementById("look-select");
const lookAttach = document.getElementById("look-attach");
const MAX_ATTACHMENTS = 3;
const MAX_IMAGE_SIDE = 1280;
let pendingImages = []; // [{ mime, data (base64), label }]
let lookCapture = null;
let attachNoteTimer = null;

// Aviso breve junto a las imágenes (por ejemplo, cuando el modelo no las admite).
function attachNote(text) {
  let note = attachmentsEl.querySelector(".attach-note");
  if (!note) {
    note = document.createElement("div");
    note.className = "attach-note";
    attachmentsEl.append(note);
  }
  note.textContent = text;
  attachmentsEl.hidden = false;
  clearTimeout(attachNoteTimer);
  attachNoteTimer = setTimeout(() => {
    note.remove();
    attachmentsEl.hidden = pendingImages.length === 0;
  }, 6000);
}

// Hasta que se cargan los ajustes se da por buena: el servidor rechaza lo que el modelo no pueda ver.
function canSeeImages() {
  const provider = settings && currentProvider();
  return !provider || provider.vision !== false;
}

function renderAttachments() {
  const note = attachmentsEl.querySelector(".attach-note");
  const chips = pendingImages.map((image, index) => {
    const chip = document.createElement("div");
    chip.className = "attach-chip";
    const thumb = document.createElement("img");
    thumb.src = `data:${image.mime};base64,${image.data}`;
    thumb.alt = image.label || "Imagen adjunta";
    thumb.title = image.label || "Imagen adjunta";
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "attach-x";
    remove.textContent = "✕";
    remove.title = remove.ariaLabel = "Quitar esta imagen";
    remove.addEventListener("click", () => {
      pendingImages.splice(index, 1);
      renderAttachments();
      input.focus();
    });
    chip.append(thumb, remove);
    return chip;
  });
  attachmentsEl.replaceChildren(...chips, ...(note ? [note] : []));
  attachmentsEl.hidden = !chips.length && !note;
}

function addPendingImage(image) {
  if (!canSeeImages()) {
    attachNote("Este proveedor todavía no puede ver imágenes. Cambia a Ollama con un modelo con visión (por ejemplo qwen3.5:9b).");
    return false;
  }
  if (pendingImages.length >= MAX_ATTACHMENTS) {
    attachNote(`Solo puedes adjuntar hasta ${MAX_ATTACHMENTS} imágenes por mensaje.`);
    return false;
  }
  pendingImages.push(image);
  renderAttachments();
  return true;
}

// Reduce la imagen (lo que se envía no necesita más de 1280 px) y la convierte a PNG; si pesa mucho, a JPEG.
async function normalizeImage(blob) {
  const bitmap = await createImageBitmap(blob);
  const scale = Math.min(1, MAX_IMAGE_SIDE / Math.max(bitmap.width, bitmap.height));
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(bitmap.width * scale));
  canvas.height = Math.max(1, Math.round(bitmap.height * scale));
  canvas.getContext("2d").drawImage(bitmap, 0, 0, canvas.width, canvas.height);
  bitmap.close?.();
  let url = canvas.toDataURL("image/png");
  let mime = "image/png";
  if (url.length > 2_000_000) {
    url = canvas.toDataURL("image/jpeg", 0.85);
    mime = "image/jpeg";
  }
  return { mime, data: url.slice(url.indexOf(",") + 1) };
}

// Ctrl+V con una imagen en el portapapeles (una captura de pantalla, por ejemplo) la adjunta al mensaje.
input.addEventListener("paste", async (e) => {
  const images = [...(e.clipboardData?.items ?? [])].filter((item) => item.kind === "file" && item.type.startsWith("image/"));
  if (!images.length) return; // texto normal: se pega como siempre
  e.preventDefault();
  for (const item of images) {
    try {
      const normalized = await normalizeImage(item.getAsFile());
      if (!addPendingImage({ ...normalized, label: "Captura pegada" })) break;
    } catch {
      attachNote("No pude leer esa imagen.");
    }
  }
});

// «Mirar una ventana»: se captura la ventana en la que estabas, se te enseña y solo se adjunta si lo aceptas.
function closeLook() {
  lookPanel.hidden = true;
  lookCapture = null;
}

async function captureLook(title) {
  lookCapture = null;
  lookAttach.disabled = true;
  lookThumb.hidden = true;
  lookCaption.textContent = "";
  lookStatus.hidden = false;
  lookStatus.textContent = "Capturando…";
  try {
    const capture = await invoke("capture_window_preview", { title: title || null });
    lookCapture = capture;
    lookThumb.src = `data:${capture.mime};base64,${capture.data}`;
    lookThumb.hidden = false;
    lookCaption.textContent = `«${capture.title}» (${capture.process}) · ${capture.width}×${capture.height}`;
    lookStatus.hidden = true;
    lookAttach.disabled = false;
  } catch (error) {
    lookStatus.textContent = String(error);
  }
}

async function openLook() {
  if (!canSeeImages()) {
    attachNote("Este proveedor todavía no puede ver imágenes. Cambia a Ollama con un modelo con visión (por ejemplo qwen3.5:9b).");
    return;
  }
  hidePalette();
  closeModelMenu();
  lookPanel.hidden = false;
  lookSelect.replaceChildren(new Option("La ventana en la que estabas", ""));
  try {
    const windows = await invoke("list_capturable_windows");
    for (const w of windows) lookSelect.append(new Option(`${w.title} — ${w.process}`, w.title));
  } catch {
    // sin la lista se puede seguir con la ventana automática
  }
  captureLook("");
}

lookBtn.addEventListener("click", () => (lookPanel.hidden ? openLook() : closeLook()));
document.getElementById("look-close").addEventListener("click", closeLook);
document.getElementById("look-cancel").addEventListener("click", closeLook);
lookSelect.addEventListener("change", () => captureLook(lookSelect.value));
lookAttach.addEventListener("click", () => {
  if (!lookCapture) return;
  const { mime, data, title } = lookCapture;
  if (addPendingImage({ mime, data, label: title })) {
    closeLook();
    input.focus();
  }
});

// --- Fechas legibles de cosas que ya pasaron ---
const capital = (text) => `${text.charAt(0).toUpperCase()}${text.slice(1)}`;

// «Hoy 21:05», «Ayer 18:20», «Lunes 20:10», «2 nov», «2 nov 2025».
function pastLabel(seconds) {
  const date = new Date(seconds * 1000);
  const now = new Date();
  const dayStart = (d) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((dayStart(now) - dayStart(date)) / 86400000);
  const time = date.toLocaleTimeString("es", { hour: "2-digit", minute: "2-digit" });
  if (days <= 0) return `Hoy ${time}`;
  if (days === 1) return `Ayer ${time}`;
  if (days < 7) return `${capital(date.toLocaleDateString("es", { weekday: "long" }))} ${time}`;
  const sameYear = date.getFullYear() === now.getFullYear();
  return date.toLocaleDateString("es", sameYear ? { day: "numeric", month: "short" } : { day: "numeric", month: "short", year: "numeric" });
}

// --- Hoy: lo importante de un vistazo ---
const todayDate = document.getElementById("today-date");
const todayGreeting = document.getElementById("today-greeting");
const todayMediaCard = document.getElementById("today-media-card");
const todayMediaTitle = document.getElementById("today-media-title");
const todayMediaSub = document.getElementById("today-media-sub");
const todayMediaPrev = document.getElementById("today-media-prev");
const todayMediaToggle = document.getElementById("today-media-toggle");
const todayMediaNext = document.getElementById("today-media-next");
const todayRemindersEl = document.getElementById("today-reminders");
const todayRemindersEmpty = document.getElementById("today-reminders-empty");
const todayAllReminders = document.getElementById("today-all-reminders");
const todayStats = document.getElementById("today-stats");
const todayLastCard = document.getElementById("today-last-card");
const todayLastTitle = document.getElementById("today-last-title");
const todayLastSub = document.getElementById("today-last-sub");
const todayLastResume = document.getElementById("today-last-resume");
const todayShortcuts = document.getElementById("today-shortcuts");
let todayTimer = null;
let lastMedia = null;
let lastConversationId = null;

function renderTodayHeader() {
  const now = new Date();
  const hour = now.getHours();
  const hello = hour >= 5 && hour < 13 ? "¡Buenos días!" : hour >= 13 && hour < 20 ? "¡Buenas tardes!" : "¡Buenas noches!";
  const day = capital(now.toLocaleDateString("es", { weekday: "long", day: "numeric", month: "long" }));
  todayDate.textContent = `${day} · ${now.toLocaleTimeString("es", { hour: "2-digit", minute: "2-digit" })}`;
  todayGreeting.textContent = `${hello} ¿En qué te ayudo hoy?`;
}

// Los tres recordatorios más próximos; el resto, en su pestaña.
function renderTodayReminders() {
  todayRemindersEl.replaceChildren(...reminders.slice(0, 3).map(reminderRow));
  todayRemindersEmpty.hidden = reminders.length > 0;
  todayAllReminders.hidden = reminders.length <= 3;
  todayAllReminders.textContent = `Ver todos (${reminders.length})`;
  refreshCountdowns(todayRemindersEl);
}
todayAllReminders.addEventListener("click", () => showTab("reminders"));

function renderTodayMedia() {
  todayMediaCard.hidden = !lastMedia;
  if (!lastMedia) return;
  const playing = lastMedia.state === "playing";
  todayMediaTitle.textContent = lastMedia.title || "Sin título";
  todayMediaSub.textContent = [lastMedia.artist, lastMedia.app].filter(Boolean).join(" · ");
  todayMediaToggle.textContent = playing ? "⏸" : "▶";
  todayMediaToggle.title = todayMediaToggle.ariaLabel = playing ? "Pausar" : "Reproducir";
  todayMediaPrev.disabled = !lastMedia.can_previous;
  todayMediaNext.disabled = !lastMedia.can_next;
  todayMediaToggle.disabled = !lastMedia.can_play_pause;
}

async function refreshMedia() {
  try {
    const snapshot = await invoke("get_media");
    lastMedia = snapshot ? snapshot.state : null;
  } catch {
    lastMedia = null;
  }
  renderTodayMedia();
}
window.__TAURI__.event.listen("media-state", (e) => {
  lastMedia = e.payload;
  renderTodayMedia();
});

async function pressMedia(action) {
  try {
    await invoke("media_control", { action });
  } catch {
    // la aplicación puede no aceptar la orden; el estado real se vuelve a leer enseguida
  }
  setTimeout(refreshMedia, 300);
}
todayMediaPrev.addEventListener("click", () => pressMedia("previous"));
todayMediaToggle.addEventListener("click", () => pressMedia("play_pause"));
todayMediaNext.addEventListener("click", () => pressMedia("next"));

const gigabytes = (bytes) => `${(bytes / 2 ** 30).toFixed(1).replace(".", ",")} GB`;

function meter(label, percent, detail, warn) {
  const box = document.createElement("div");
  box.className = "meter";
  const top = document.createElement("div");
  top.className = "meter-top";
  const name = document.createElement("span");
  name.textContent = label;
  const value = document.createElement("strong");
  value.textContent = detail;
  top.append(name, value);
  const bar = document.createElement("div");
  bar.className = `meter-bar${warn ? " warn" : ""}`;
  const fill = document.createElement("i");
  fill.style.width = `${Math.max(0, Math.min(100, percent))}%`;
  bar.append(fill);
  box.append(top, bar);
  return box;
}

async function refreshStatus() {
  try {
    const s = await invoke("get_system_status");
    const rows = [];
    if (s.battery) {
      const low = s.battery.percent <= 20 && !s.battery.charging;
      rows.push(meter(`🔋 Batería${s.battery.charging ? " · cargando" : ""}`, s.battery.percent, `${s.battery.percent} %`, low));
    }
    rows.push(meter("🧠 Memoria", s.memory_used_percent, `${s.memory_used_percent} % de ${gigabytes(s.memory_total_bytes)}`, s.memory_used_percent >= 90));
    if (s.disk_total_bytes > 0) {
      const usedPercent = Math.round(((s.disk_total_bytes - s.disk_free_bytes) / s.disk_total_bytes) * 100);
      rows.push(meter("💾 Disco", usedPercent, `${gigabytes(s.disk_free_bytes)} libres de ${gigabytes(s.disk_total_bytes)}`, usedPercent >= 90));
    }
    const hours = Math.floor(s.uptime_seconds / 3600);
    const uptime = document.createElement("p");
    uptime.className = "hint";
    uptime.textContent = hours >= 24 ? `Encendido desde hace ${Math.floor(hours / 24)} d ${hours % 24} h.` : `Encendido desde hace ${hours} h ${Math.floor((s.uptime_seconds % 3600) / 60)} min.`;
    todayStats.replaceChildren(...rows, uptime);
  } catch {
    todayStats.textContent = "No pude leer el estado del equipo.";
  }
}

async function refreshLastConversation() {
  try {
    const [latest] = await invoke("list_conversations", {});
    todayLastCard.hidden = !latest;
    if (!latest) return;
    lastConversationId = latest.id;
    todayLastTitle.textContent = latest.title;
    todayLastSub.textContent = `${pastLabel(latest.updated)} · ${latest.count} mensajes`;
  } catch {
    todayLastCard.hidden = true;
  }
}
todayLastResume.addEventListener("click", () => resumeConversation(lastConversationId));

// Lo que se pide con un clic: se envía tal cual, salvo los atajos que necesitan completarse.
const SHORTCUTS = [
  ["🎵", "Pon música tranquila", true],
  ["📂", "¿Qué ocupa más espacio en mis Descargas?", true],
  ["🔁", "Busca archivos duplicados en el escritorio", true],
  ["🗂️", "Organiza mi carpeta de Descargas", true],
  ["🪟", "Mira esta ventana y dime qué ves", true],
  ["⏰", "Recuérdame ", false],
  ["🆕", "/new", true],
];

function askRyan(text, send) {
  showTab("chat");
  if (!send) {
    input.value = text;
    input.focus();
    input.setSelectionRange(text.length, text.length);
    return;
  }
  hideIdeas();
  remember(text);
  addMessage("user", text);
  invoke("submit_message", { text });
}

// Cada modo propone lo que más encaja con él.
const PROFILE_SHORTCUTS = {
  work: [
    ["📂", "¿Qué ocupa más espacio en mis Descargas?", true],
    ["🔁", "Busca archivos duplicados en el escritorio", true],
    ["🗂️", "Organiza mi carpeta de Descargas", true],
    ["🪟", "Mira esta ventana y dime qué ves", true],
    ["⏰", "Recuérdame ", false],
    ["🆕", "/new", true],
  ],
  rest: [
    ["🎵", "Pon música tranquila", true],
    ["😄", "Cuéntame un chiste", true],
    ["⏰", "Recuérdame descansar en 15 minutos", true],
    ["🔎", "Busca algo interesante para hoy", true],
    ["🎲", "Propón algo divertido que hacer", true],
    ["🆕", "/new", true],
  ],
  focus: [
    ["⏰", "Recuérdame ", false],
    ["🪟", "Mira esta ventana y dime qué ves", true],
    ["🆕", "/new", true],
  ],
};

function renderShortcuts() {
  const list = PROFILE_SHORTCUTS[settings?.profile] ?? SHORTCUTS;
  todayShortcuts.replaceChildren(
    ...list.map(([icon, text, send]) => {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "idea";
      button.textContent = `${icon} ${text.trim()}${send ? "" : "…"}`;
      button.addEventListener("click", () => askRyan(text, send));
      return button;
    }),
  );
}
renderShortcuts();

// --- Modos (perfiles) ---
const profileChips = document.getElementById("profile-chips");
const profileDesc = document.getElementById("profile-desc");
const profileChanges = document.getElementById("profile-changes");
const profileSelect = document.getElementById("profile-select");
const profileSelectDesc = document.getElementById("profile-select-desc");
const profileBadge = document.getElementById("profile-badge");

function renderProfile() {
  if (!settings?.profiles) return;
  const active = settings.profiles.find((p) => p.id === settings.profile) ?? settings.profiles[0];
  profileChips.replaceChildren(
    ...settings.profiles.map((p) => {
      const chip = document.createElement("button");
      chip.type = "button";
      chip.className = "profile-chip";
      chip.setAttribute("role", "radio");
      chip.setAttribute("aria-checked", String(p.id === active.id));
      chip.textContent = `${p.icon} ${p.name}`;
      chip.title = p.description;
      chip.addEventListener("click", () => chooseProfile(p.id));
      return chip;
    }),
  );
  profileDesc.textContent = active.description;
  profileChanges.replaceChildren(
    ...active.changes.map((text) => {
      const item = document.createElement("li");
      item.textContent = text;
      return item;
    }),
  );
  if (profileSelect.options.length !== settings.profiles.length) {
    profileSelect.replaceChildren(...settings.profiles.map((p) => new Option(`${p.icon} ${p.name}`, p.id)));
  }
  profileSelect.value = active.id;
  profileSelectDesc.textContent = active.description;
  // Solo se destaca en la cabecera cuando no es el modo Normal.
  profileBadge.hidden = active.id === "normal";
  profileBadge.textContent = `${active.icon} ${active.name}`;
  renderShortcuts();
}

async function chooseProfile(id) {
  if (!settings || id === settings.profile) return;
  try {
    await invoke("set_profile", { id });
    await loadSettings();
  } catch (error) {
    addMessage("assistant", `No pude cambiar de modo: ${error}`);
  }
}
profileSelect.addEventListener("change", () => chooseProfile(profileSelect.value));
profileBadge.addEventListener("click", () => showTab("today"));
window.__TAURI__.event.listen("profile-changed", () => loadSettings().catch(() => {}));

async function refreshToday() {
  await Promise.allSettled([loadReminders(), refreshStatus(), refreshMedia(), refreshLastConversation()]);
  renderTodayHeader();
}

function openToday() {
  renderTodayHeader();
  refreshToday();
  clearInterval(todayTimer);
  let ticks = 0;
  todayTimer = setInterval(() => {
    refreshCountdowns(todayRemindersEl);
    if (++ticks % 10 === 0) refreshToday();
  }, 1000);
}

function closeToday() {
  clearInterval(todayTimer);
  todayTimer = null;
}

// --- Historial de conversaciones ---
const historyItems = document.getElementById("history-items");
const historyEmpty = document.getElementById("history-empty");
const historySearch = document.getElementById("history-search");
const historySave = document.getElementById("history-save");
const historyClear = document.getElementById("history-clear");
const historyMessage = document.getElementById("history-message");
const historyListView = document.getElementById("history-list-view");
const historyDetailView = document.getElementById("history-detail-view");
const historyBack = document.getElementById("history-back");
const historyResume = document.getElementById("history-resume");
const historyDelete = document.getElementById("history-delete");
const detailTitle = document.getElementById("detail-title");
const detailMeta = document.getElementById("detail-meta");
const detailMessages = document.getElementById("detail-messages");
const detailMessage = document.getElementById("detail-message");
let openConversationId = null;
let searchTimer = null;

// Cada carácter en minúscula y sin tilde, uno por uno: así los índices coinciden con el texto original.
const foldChars = (text) => [...text].map((c) => c.normalize("NFD").charAt(0).toLowerCase());

// Escribe `text` en `el` resaltando lo que coincide con la búsqueda (sin distinguir mayúsculas ni tildes).
function appendHighlighted(el, text, query) {
  const needle = foldChars(query.trim());
  if (!needle.length) {
    el.textContent = text;
    return;
  }
  const chars = [...text];
  const folded = foldChars(text);
  let from = 0;
  while (from < chars.length) {
    let found = -1;
    for (let start = from; start + needle.length <= folded.length; start += 1) {
      if (needle.every((c, offset) => folded[start + offset] === c)) {
        found = start;
        break;
      }
    }
    if (found < 0) break;
    el.append(chars.slice(from, found).join(""));
    const mark = document.createElement("mark");
    mark.textContent = chars.slice(found, found + needle.length).join("");
    el.append(mark);
    from = found + needle.length;
  }
  el.append(chars.slice(from).join(""));
}

function conversationCard(item, query) {
  const card = document.createElement("button");
  card.type = "button";
  card.className = "conv";
  const title = document.createElement("div");
  title.className = "conv-title";
  appendHighlighted(title, item.title, query);
  const meta = document.createElement("div");
  meta.className = "conv-meta";
  meta.textContent = `${pastLabel(item.updated)} · ${item.count} mensajes`;
  const preview = document.createElement("div");
  preview.className = "conv-preview";
  appendHighlighted(preview, item.snippet || item.preview, query);
  card.append(title, meta, preview);
  card.addEventListener("click", () => openConversation(item.id));
  return card;
}

async function loadHistory() {
  const query = historySearch.value.trim();
  try {
    const items = await invoke("list_conversations", { query: query || null });
    historyItems.replaceChildren(...items.map((item) => conversationCard(item, query)));
    historyEmpty.hidden = items.length > 0;
    if (!items.length) {
      historyEmpty.textContent = query
        ? `Ninguna conversación menciona «${query}».`
        : historySave.checked
          ? "Todavía no hay conversaciones guardadas. Habla con tu asistente en la pestaña Charla y aparecerán aquí."
          : "El historial está apagado: las conversaciones nuevas no se guardan.";
    }
  } catch (error) {
    setMessage(historyMessage, String(error), "error");
  }
}

function openHistory() {
  historyListView.hidden = false;
  historyDetailView.hidden = true;
  loadHistory();
}

historySearch.addEventListener("input", () => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(loadHistory, 180);
});

async function openConversation(id) {
  try {
    const conversation = await invoke("load_conversation", { id });
    openConversationId = id;
    detailTitle.textContent = conversation.title;
    detailMeta.textContent = `${pastLabel(conversation.created)} · ${conversation.messages.length} mensajes`;
    detailMessages.replaceChildren(...conversation.messages.map((m) => buildMessage(m.role, m.text, m.at)));
    setMessage(detailMessage, "");
    historyDelete.textContent = "Borrar";
    historyListView.hidden = true;
    historyDetailView.hidden = false;
    document.querySelector("#view-history .panels").scrollTop = 0;
  } catch (error) {
    setMessage(historyMessage, String(error), "error");
    loadHistory();
  }
}
historyBack.addEventListener("click", openHistory);

// Vuelve a mostrar la conversación en el chat y la IA recupera lo hablado; lo nuevo se añade a ella.
async function resumeConversation(id) {
  try {
    const conversation = await invoke("resume_conversation", { id });
    hideIdeas();
    history.replaceChildren();
    typingEl = null;
    streamEl = null;
    for (const m of conversation.messages) addToHistory(buildMessage(m.role, m.text, m.at));
    const divider = document.createElement("div");
    divider.className = "divider";
    divider.textContent = `Continúas la conversación «${conversation.title}»`;
    addToHistory(divider);
    renderTyping();
    showTab("chat");
  } catch (error) {
    setMessage(openConversationId === id ? detailMessage : historyMessage, String(error), "error");
  }
}
historyResume.addEventListener("click", () => resumeConversation(openConversationId));

// Borrar pide una segunda pulsación: así no se pierde una conversación por un clic suelto.
function armed(button, idleText, onConfirm) {
  let timer = null;
  button.addEventListener("click", () => {
    if (timer) {
      clearTimeout(timer);
      timer = null;
      button.textContent = idleText;
      onConfirm();
      return;
    }
    button.textContent = "¿Seguro? Pulsa otra vez";
    timer = setTimeout(() => {
      timer = null;
      button.textContent = idleText;
    }, 3500);
  });
}

armed(historyDelete, "Borrar", async () => {
  try {
    await invoke("delete_conversation", { id: openConversationId });
    openHistory();
    setMessage(historyMessage, "Conversación borrada.", "ok");
  } catch (error) {
    setMessage(detailMessage, String(error), "error");
  }
});

armed(historyClear, "Borrar todo el historial", async () => {
  try {
    await invoke("clear_conversations");
    setMessage(historyMessage, "Historial borrado.", "ok");
  } catch (error) {
    setMessage(historyMessage, String(error), "error");
  }
  loadHistory();
});

historySave.addEventListener("change", async () => {
  try {
    await invoke("set_save_history", { value: historySave.checked });
    setMessage(historyMessage, historySave.checked ? "Las conversaciones nuevas se guardarán." : "Las conversaciones nuevas ya no se guardan. Las anteriores siguen ahí.", "ok");
  } catch (error) {
    setMessage(historyMessage, String(error), "error");
  }
  await loadSettings().catch(() => {});
  loadHistory();
});

// Al arrancar, la página puede cargar antes de que la aplicación termine de prepararse: se reintenta unos segundos.
async function loadSettingsWhenReady(attempt = 0) {
  try {
    await loadSettings();
  } catch {
    if (attempt < 30) setTimeout(() => loadSettingsWhenReady(attempt + 1), 400);
  }
}
loadSettingsWhenReady();
loadHeaderAvatar();
