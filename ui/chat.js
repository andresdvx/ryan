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

function addMessage(role, text) {
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
  time.textContent = new Date().toLocaleTimeString("es", { hour: "2-digit", minute: "2-digit" });
  el.append(time);
  addToHistory(el);
}

function renderTyping() {
  // Mientras se espera la respuesta del usuario a una confirmación no se aparenta estar ocupado.
  const show = busyState !== null && pendingConfirmations === 0;
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
  typingEl.querySelector(".label").textContent = busyState === "working" ? "Trabajando…" : "Pensando…";
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
  if (!text) return;
  remember(text);
  recallIndex = null;
  input.value = "";
  // La API key no debe quedar a la vista en el historial.
  addMessage("user", text.startsWith("/key ") ? "/key ••••••••" : text);
  invoke("submit_message", { text });
});

document.getElementById("close").addEventListener("click", () => invoke("hide_chat"));
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") invoke("hide_chat");
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
  input.placeholder = `Pídele algo a ${name}…`;
}

invoke("get_assistant_name")
  .then(applyName)
  .catch(() => {})
  .finally(() => {
    addMessage("assistant", `Soy ${assistantName}. ¿En qué te puedo ayudar?`);
  });
window.__TAURI__.event.listen("assistant-name-changed", (e) => applyName(e.payload));
const STATUS_TEXT = {
  idle: "Listo", walking: "Paseando", sleeping: "Durmiendo", thinking: "Pensando…", working: "Trabajando…",
  success: "¡Hecho!", happy: "Contento", surprised: "Algo salió mal", interacting: "Listo", alert: "¡Recordatorio!",
};
window.__TAURI__.event.listen("character-state", (e) => {
  statusEl.textContent = STATUS_TEXT[e.payload] ?? e.payload;
  const state = e.payload;
  // Entre dos pasos de una misma tarea no se aparenta haber terminado.
  const midTask = state === "success" && taskProgress && taskProgress.step < taskProgress.total;
  if (!midTask && state !== "thinking" && state !== "working" && state !== "success") taskProgress = null;
  setBusy(midTask ? "working" : state === "thinking" || state === "working" ? state : null);
});
window.__TAURI__.event.listen("chat-log", (e) => addMessage("assistant", e.payload));
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

async function loadHeaderAvatar() {
  try {
    characterManifest ??= await fetch("assets/characters.json").then((response) => response.json());
    const id = await invoke("get_character");
    const entry = characterManifest.characters.find((c) => c.id === id) ?? characterManifest.characters[0];
    headerAvatar.src = previewUrl(entry);
    headerAvatar.hidden = false;
  } catch {
    headerAvatar.hidden = true;
  }
}
window.__TAURI__.event.listen("character-changed", loadHeaderAvatar);

// --- Pestañas ---
const tabs = document.querySelectorAll(".tab");
const views = { chat: document.getElementById("view-chat"), settings: document.getElementById("view-settings") };

function showTab(name) {
  tabs.forEach((tab) => {
    const active = tab.dataset.tab === name;
    tab.classList.toggle("active", active);
    tab.setAttribute("aria-selected", String(active));
  });
  Object.entries(views).forEach(([key, view]) => (view.hidden = key !== name));
  if (name === "settings") loadSettings();
  else input.focus();
}
tabs.forEach((tab) => tab.addEventListener("click", () => showTab(tab.dataset.tab)));

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
  autostartBox.checked = settings.autostart;
  renderAvatars();
  nameInput.value = settings.assistant_name;
  wanderBox.checked = settings.wander;
  lookBox.checked = settings.look_at_cursor;
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
let appsData = null;

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
  const shown = appsData.apps.filter((app) => app.name.toLowerCase().includes(needle));
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

// Los personajes se describen en assets/characters.json, el mismo archivo que usa la mascota.
let characterManifest = null;

function previewUrl(entry) {
  const n = entry.preview_frame;
  const name = entry.pattern.replace("{nn}", String(n).padStart(2, "0")).replace("{n}", String(n));
  return `assets/${entry.folder}/${name}`;
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

// Qué herramientas dejó desplegadas el usuario (se conserva al volver a dibujar la lista).
const openTools = new Set();

// Cada herramienta es un acordeón: a la vista solo su nombre y su casilla; la descripción, al desplegarla.
function renderTools() {
  toolsList.replaceChildren(
    ...settings.tools.map((tool) => {
      const details = document.createElement("details");
      details.className = "tool";
      details.open = openTools.has(tool.name);
      details.addEventListener("toggle", () => (details.open ? openTools.add(tool.name) : openTools.delete(tool.name)));

      const summary = document.createElement("summary");
      const box = document.createElement("input");
      box.type = "checkbox";
      box.checked = tool.enabled;
      box.setAttribute("aria-label", `Permitir ${tool.name}`);
      // Marcar la casilla no debe desplegar ni plegar la descripción.
      box.addEventListener("click", (event) => event.stopPropagation());
      box.addEventListener("change", () => changeSetting(toolsMessage, "set_tool_enabled", { name: tool.name, enabled: box.checked }));
      const name = document.createElement("span");
      name.className = "name";
      name.textContent = tool.name;
      summary.append(box, name);

      const desc = document.createElement("p");
      desc.className = "desc";
      desc.textContent = tool.description;
      details.append(summary, desc);
      return details;
    }),
  );
}

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
loadHeaderAvatar();
