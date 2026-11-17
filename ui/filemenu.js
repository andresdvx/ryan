// Menú de archivos: aparece al soltar un archivo sobre el chat o el personaje. Analizar, resumir y preguntar se
// envían a la IA a través del chat; convertir guarda el resultado en la carpeta elegida (el Escritorio por defecto).
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const menu = $("menu");
const mainView = $("fm-main");
const statusEl = $("fm-status");
const doneEl = $("fm-done");
const askForm = $("fm-ask");
const askInput = $("fm-q");
const askToggle = $("fm-ask-toggle");
const noteEl = $("fm-note");

const ICONS = {
  image: "🖼️", pdf: "📕", docx: "📘", pptx: "📙", spreadsheet: "📗", html: "🌐", markdown: "📝", csv: "🧾", json: "🧾",
  text: "📝", code: "💻", zip: "🗜️", folder: "📁", other: "📄",
};

let state = null;
let busy = false;

// La ventana es transparente: su altura se ajusta a lo que se muestra para no tapar nada de más.
function fit() {
  invoke("file_menu_resize", { height: Math.ceil(menu.getBoundingClientRect().height) + 20 }).catch(() => {});
}
new ResizeObserver(fit).observe(menu);

function formatSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value >= 10 || unit === 0 && value >= 100 ? Math.round(value) : value.toFixed(1).replace(".", ",")} ${units[unit]}`;
}

function show(view) {
  mainView.hidden = view !== "main";
  statusEl.hidden = view !== "status";
  doneEl.hidden = view !== "done";
}

function setStatus(text, { error = false, retry = null } = {}) {
  statusEl.className = error ? "status error" : "status";
  statusEl.replaceChildren();
  if (error) {
    const message = document.createElement("div");
    message.textContent = text;
    statusEl.append(message);
    const back = document.createElement("button");
    back.type = "button";
    back.className = "act small";
    back.textContent = "↩ Volver";
    back.addEventListener("click", () => show("main"));
    statusEl.append(back);
  } else {
    const spinner = document.createElement("span");
    spinner.className = "spinner";
    const label = document.createElement("span");
    label.textContent = text;
    statusEl.append(spinner, label);
  }
  show("status");
}

function setOutput(output) {
  const dir = $("fm-dir");
  dir.textContent = `${output.custom ? "📁" : "🖥"} ${output.label}`;
  dir.title = `${output.path}\n(pulsa para cambiarla)`;
  $("fm-dir-reset").hidden = !output.custom;
}

function chip(offer) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "chip";
  button.textContent = offer.label;
  button.dataset.id = offer.id;
  button.addEventListener("click", () => convert(offer));
  return button;
}

function render(data) {
  state = data;
  busy = false;
  // Soltar sobre «Convertir» de la barra abre solo el formulario de conversiones; el resto va por el chat.
  $("fm-ai-block").hidden = data.mode === "convert";
  const [first, ...rest] = data.files;
  const kinds = new Set(data.files.map((f) => f.type));
  $("fm-icon").textContent = data.files.length === 1 ? (ICONS[first.type] ?? "📄") : kinds.size === 1 ? (ICONS[first.type] ?? "📄") : "🗂️";
  $("fm-name").textContent = rest.length ? `${first.name} y ${rest.length} más` : first.name;
  const total = data.files.reduce((sum, f) => sum + f.size, 0);
  $("fm-meta").textContent = rest.length
    ? `${data.files.length} archivos${total ? ` · ${formatSize(total)}` : ""}`
    : first.folder ? first.kind : `${first.kind} · ${formatSize(first.size)}`;

  for (const id of ["fm-analyze", "fm-summarize", "fm-ask-toggle"]) $(id).disabled = !data.ask.enabled;
  askForm.hidden = true;
  askToggle.setAttribute("aria-expanded", "false");
  askInput.value = "";
  noteEl.className = "note";
  noteEl.textContent = data.ask.reason
    ?? data.ask.hint
    ?? (data.provider.local ? `Se queda en tu equipo: ${data.provider.name} funciona aquí.` : `Analizar, resumir y preguntar envían el contenido a ${data.provider.name}.`);

  const convertOffers = data.offers.filter((o) => o.group === "convert");
  const optimizeOffers = data.offers.filter((o) => o.group === "optimize");
  $("fm-convert").replaceChildren(...convertOffers.map(chip));
  $("fm-optimize").replaceChildren(...optimizeOffers.map(chip));
  $("fm-convert-title").classList.toggle("empty", convertOffers.length === 0);
  $("fm-optimize-title").classList.toggle("empty", optimizeOffers.length === 0);
  setOutput(data.output);
  show("main");
}

async function ask(action, question = null) {
  if (busy) return;
  busy = true;
  setStatus(action === "ask" ? "Mandándole tu pregunta…" : action === "analyze" ? "Preparando el análisis…" : "Preparando el resumen…");
  try {
    await invoke("file_menu_ai", { action, question });
    // La ventana se cierra sola: la respuesta aparece en el chat.
  } catch (error) {
    setStatus(String(error), { error: true });
  } finally {
    busy = false;
  }
}

async function convert(offer) {
  if (busy) return;
  busy = true;
  setStatus(`Convirtiendo a ${offer.label}…`);
  try {
    const result = await invoke("file_menu_convert", { target: offer.id });
    $("fm-done-title").textContent = result.outputs.length === 1 ? "✔ Listo, guardado en " + result.dir : `✔ Listo: ${result.outputs.length} archivos en ${result.dir}`;
    $("fm-done-list").replaceChildren(...result.outputs.map((name) => {
      const item = document.createElement("li");
      item.textContent = name;
      item.title = name;
      return item;
    }));
    $("fm-done-notes").textContent = (result.notes ?? []).join(" ");
    $("fm-done-failures").textContent = result.failures.length ? `No se pudo convertir: ${result.failures.join(" ")}` : "";
    show("done");
  } catch (error) {
    setStatus(String(error), { error: true });
  } finally {
    busy = false;
  }
}

$("fm-analyze").addEventListener("click", () => ask("analyze"));
$("fm-summarize").addEventListener("click", () => ask("summarize"));
askToggle.addEventListener("click", () => {
  const open = askForm.hidden;
  askForm.hidden = !open;
  askToggle.setAttribute("aria-expanded", String(open));
  if (open) askInput.focus();
});
askForm.addEventListener("submit", (e) => {
  e.preventDefault();
  const question = askInput.value.trim();
  if (!question) return;
  ask("ask", question);
});

$("fm-dir-change").addEventListener("click", changeDir);
$("fm-dir").addEventListener("click", changeDir);
async function changeDir() {
  try {
    const output = await invoke("file_menu_choose_dir");
    if (output) setOutput(output);
  } catch (error) {
    noteEl.className = "note error";
    noteEl.textContent = String(error);
  }
}
$("fm-dir-reset").addEventListener("click", async () => setOutput(await invoke("file_menu_reset_dir")));

$("fm-reveal").addEventListener("click", () => invoke("file_menu_reveal").catch(() => {}));
$("fm-again").addEventListener("click", () => show("main"));
$("fm-done-close").addEventListener("click", close);
$("fm-close").addEventListener("click", close);
function close() {
  invoke("file_menu_close").catch(() => {});
}
document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape") return;
  if (!askForm.hidden && document.activeElement === askInput) {
    askForm.hidden = true;
    askToggle.setAttribute("aria-expanded", "false");
  } else {
    close();
  }
});

listen("file-menu-open", (e) => render(e.payload));
invoke("file_menu_state").then((data) => data?.files?.length && render(data)).catch(() => {});

// El color del menú sigue al personaje elegido, igual que el chat y la barra rápida.
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
listen("file-menu-open", applyTheme);
