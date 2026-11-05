// Barra rápida: pides algo sin abrir el chat y ves la respuesta aquí. Todo lo que se escribe sigue el mismo
// camino que el chat (permisos, confirmaciones, historial); si algo necesita tu permiso, se abre el chat.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const appWindow = window.__TAURI__.window.getCurrentWindow();

const bar = document.getElementById("bar");
const input = document.getElementById("q-input");
const stateEl = document.getElementById("q-state");
const answer = document.getElementById("q-answer");
const foot = document.getElementById("q-foot");

// Hay una petición en curso (se envió y aún no terminó de contestar).
let waiting = false;
let streamEl = null;
let streamText = "";
let doneTimer = null;

// La ventana es transparente: su altura se ajusta a lo que se muestra para no tapar nada de más.
function fit() {
  invoke("resize_quick", { height: Math.ceil(bar.getBoundingClientRect().height) + 20 }).catch(() => {});
}

function setState(text, busy = false) {
  stateEl.textContent = text;
  stateEl.classList.toggle("busy", busy);
}

function reset() {
  clearTimeout(doneTimer);
  waiting = false;
  streamEl = null;
  streamText = "";
  answer.replaceChildren();
  answer.hidden = true;
  foot.hidden = true;
  input.value = "";
  setState("");
  fit();
  input.focus();
}

function show() {
  answer.hidden = false;
  fit();
  answer.scrollTop = answer.scrollHeight;
}

function note(text, error = false) {
  const el = document.createElement("div");
  el.className = `note${error ? " error" : ""}`;
  el.textContent = text;
  answer.append(el);
  show();
}

function startStream() {
  streamEl = document.createElement("div");
  streamEl.className = "reply streaming";
  streamText = "";
  answer.append(streamEl);
}

function stream(delta) {
  if (!streamEl) startStream();
  streamText += delta;
  streamEl.textContent = streamText;
  show();
}

// El mensaje completo sustituye a lo escrito en directo, ya con formato.
function finalMessage(text) {
  const reply = document.createElement("div");
  reply.className = "reply";
  if (window.renderRich) {
    reply.classList.add("rich");
    window.renderRich(reply, text, (url) => invoke("open_link", { url }).catch(() => {}));
  } else {
    reply.textContent = text;
  }
  if (streamEl) streamEl.replaceWith(reply);
  else answer.append(reply);
  streamEl = null;
  streamText = "";
  show();
}

function submit() {
  const text = input.value.trim();
  if (!text) return;
  waiting = true;
  clearTimeout(doneTimer);
  answer.replaceChildren();
  const you = document.createElement("div");
  you.className = "you";
  you.textContent = text;
  answer.append(you);
  foot.hidden = true;
  setState("Pensando…", true);
  input.value = "";
  show();
  invoke("quick_submit", { text });
}

function finish() {
  if (!waiting) return;
  waiting = false;
  setState("");
  foot.hidden = false;
  fit();
}

input.addEventListener("keydown", (e) => {
  if (e.key === "Enter") {
    e.preventDefault();
    submit();
  }
});
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") invoke("hide_quick_cmd");
});
document.getElementById("q-close").addEventListener("click", () => invoke("hide_quick_cmd"));
document.getElementById("q-open").addEventListener("click", () => invoke("open_chat_from_quick"));

// Al perder el foco se esconde, salvo que esté esperando una respuesta.
appWindow.onFocusChanged(({ payload: focused }) => {
  if (!focused && !waiting) invoke("hide_quick_cmd");
});

listen("quick-opened", () => {
  reset();
  invoke("get_assistant_name")
    .then((name) => (input.placeholder = `Pídele algo a ${name}…`))
    .catch(() => {});
});

listen("chat-stream", (e) => {
  if (!waiting) return;
  clearTimeout(doneTimer);
  setState("Escribiendo…", true);
  stream(e.payload);
});
listen("chat-final", (e) => {
  if (waiting) finalMessage(e.payload);
});
// Los avisos de herramientas («✔ Moví…») y los errores también se muestran, pero solo mientras se espera.
listen("chat-log", (e) => {
  if (waiting) note(e.payload, e.payload.startsWith("✖") || e.payload.startsWith("No pude"));
});
listen("chat-tool", (e) => {
  if (waiting) note(e.payload.text, !e.payload.ok);
});
listen("chat-stream-abort", () => {
  streamEl?.remove();
  streamEl = null;
});
listen("confirmation-request", () => {
  if (waiting) setState("Necesita tu permiso: te lo enseño en el chat", false);
});

// Cuándo termina: la mascota vuelve al reposo y no pasa nada más durante un momento (entre dos pasos de una
// misma tarea también hay un instante de calma, por eso se espera un poco antes de darlo por terminado).
listen("character-state", (e) => {
  const state = e.payload;
  if (!waiting) return;
  if (state === "thinking" || state === "working") {
    clearTimeout(doneTimer);
    setState(state === "thinking" ? "Pensando…" : "Trabajando…", true);
  } else if (["success", "idle", "happy", "surprised"].includes(state)) {
    clearTimeout(doneTimer);
    doneTimer = setTimeout(finish, 1300);
  }
});

reset();

// El color de la barra sigue al personaje elegido, igual que el chat (los colores salen de assets/characters.json).
function hexToRgb(hex) {
  const match = /^#([0-9a-f]{6})$/i.exec(hex ?? "");
  if (!match) return null;
  const n = parseInt(match[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

async function applyTheme() {
  try {
    const manifest = await fetch("assets/characters.json").then((response) => response.json());
    const id = await invoke("get_character");
    const entry = manifest.characters.find((c) => c.id === id) ?? manifest.characters[0];
    const root = document.documentElement.style;
    const accent = hexToRgb(entry.accent);
    const accent2 = hexToRgb(entry.accent2);
    if (accent) root.setProperty("--accent-rgb", accent.join(" "));
    else root.removeProperty("--accent-rgb");
    if (accent2) root.setProperty("--accent-2-rgb", accent2.join(" "));
    else root.removeProperty("--accent-2-rgb");
  } catch {
    // sin el manifiesto se queda el violeta de siempre
  }
}
applyTheme();
listen("character-changed", applyTheme);
listen("quick-opened", applyTheme);
