// Barra de soltar: se despliega sobre el personaje en cuanto empiezas a arrastrar un archivo. Se suelta sobre una
// de sus tres zonas: analizar y resumir abren el chat; convertir abre el formulario con los tipos de conversión.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const wrap = document.getElementById("wrap");
const zones = [...document.querySelectorAll(".zone")];

// La zona que hay bajo un punto (en píxeles de la ventana), o null si no hay ninguna.
function zoneAt(point) {
  if (!point) return null;
  return document.elementFromPoint(point.x, point.y)?.closest(".zone")?.dataset.zone ?? null;
}

function highlight(zone) {
  for (const el of zones) el.classList.toggle("hot", el.dataset.zone === zone);
}

// Cada vez que se abre, la barra se despliega de nuevo (se reinicia la animación de entrada).
function reopen() {
  highlight(null);
  wrap.style.animation = "none";
  void wrap.offsetWidth;
  wrap.style.animation = "";
}

listen("dropbar-open", () => {
  reopen();
  applyTheme();
});
listen("dropbar-over", (e) => highlight(zoneAt(e.payload)));
// El archivo se soltó: la zona sobre la que cayó decide qué pasa. Fuera de las zonas se abre el menú completo.
listen("dropbar-drop", (e) => {
  const zone = zoneAt(e.payload);
  highlight(zone);
  invoke("dropbar_choose", { zone: zone ?? "none" }).catch(() => {});
});

// El color de la barra sigue al personaje elegido, igual que el chat.
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
