const pet = document.getElementById("pet");
const sprite = document.getElementById("sprite");
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// Milisegundos por frame: más rápido cuando trabaja, más lento cuando duerme.
// Cada personaje los multiplica por su propia «speed» del manifiesto.
const FRAME_MS = { idle: 110, walking: 90, thinking: 130, working: 70, success: 80, happy: 70, surprised: 90, interacting: 80, alert: 70, sleeping: 260 };

// Los personajes se describen en assets/characters.json (se copia a ui/assets al compilar).
let manifest = null;
let character = null;
let frames = [];
// Animaciones propias de un estado (opcionales, ver «states» en characters.json); si faltan se usa «standing».
let stateSets = {};
let baseFrames = [];
let frame = 0;
let frameTimer = null;
let currentState = "idle";
let loadToken = 0;

function frameUrl(entry, n) {
  const name = entry.pattern.replace("{nn}", String(n).padStart(2, "0")).replace("{n}", String(n));
  return `assets/${entry.folder}/${name}`;
}

function playAnimation(state) {
  currentState = state;
  clearInterval(frameTimer);
  if (!character) return;
  const own = stateSets[state];
  const next = own?.length ? own : baseFrames;
  if (next !== frames) {
    frames = next;
    frame = 0;
    sprite.src = frames[0].src;
  }
  const ms = (FRAME_MS[state] ?? FRAME_MS.idle) * (character.speed ?? 1);
  frameTimer = setInterval(() => {
    frame = (frame + 1) % frames.length;
    sprite.src = frames[frame].src;
  }, ms);
}

async function useCharacter(id) {
  const token = ++loadToken;
  manifest ??= await fetch("assets/characters.json").then((response) => response.json());
  const entry =
    manifest.characters.find((c) => c.id === id) ??
    manifest.characters.find((c) => c.id === manifest.default) ??
    manifest.characters[0];

  const load = (spec) =>
    Array.from({ length: spec.frames }, (_, i) => {
      const img = new Image();
      img.src = frameUrl(spec, i + 1);
      return img;
    });
  const images = load(entry);
  const sets = Object.fromEntries(Object.entries(entry.states ?? {}).map(([state, spec]) => [state, load(spec)]));
  // Se precargan todos los frames antes de cambiar, para que no haya parpadeos ni huecos.
  await Promise.all([...images, ...Object.values(sets).flat()].map((img) => img.decode().catch(() => {})));
  if (token !== loadToken) return; // se pidió otro personaje mientras cargaba: gana el último
  // Una animación de estado que no cargó (archivo ausente) se descarta: ese estado usa «standing».
  for (const [state, set] of Object.entries(sets)) {
    if (!set.every((img) => img.naturalWidth > 0)) delete sets[state];
  }

  character = entry;
  baseFrames = images;
  stateSets = sets;
  frames = images;
  frame = 0;
  sprite.src = frames[0].src;
  pet.dataset.character = entry.id;
  playAnimation(currentState);
}

invoke("get_character")
  .then(useCharacter)
  .catch(() => useCharacter(null));
listen("character-changed", (e) => useCharacter(e.payload));

const appWindow = window.__TAURI__.window.getCurrentWindow();
const DRAG_THRESHOLD = 4;

let down = null;
let dragged = false;

// Arrastrar la mascota con el botón izquierdo; si no se mueve, cuenta como click.
pet.addEventListener("mousedown", (e) => {
  if (e.button !== 0) return;
  down = { x: e.screenX, y: e.screenY };
  dragged = false;
});

window.addEventListener("mousemove", (e) => {
  if (!down || dragged) return;
  if (Math.hypot(e.screenX - down.x, e.screenY - down.y) > DRAG_THRESHOLD) {
    dragged = true;
    invoke("pet_dragged");
    appWindow.startDragging();
  }
});

window.addEventListener("mouseup", () => {
  if (down && !dragged) onPetClicked();
  down = null;
});

function onPetClicked() {
  invoke("toggle_chat_cmd");
  pet.classList.remove("bounce");
  void pet.offsetWidth; // reinicia la animación
  pet.classList.add("bounce");
}

pet.addEventListener("animationend", (e) => {
  if (e.animationName === "bounce") pet.classList.remove("bounce");
});

// El estado lo decide el backend (máquina de estados); aquí solo se refleja en el DOM.
listen("character-state", (e) => {
  if (e.payload !== "alert") delete pet.dataset.alertKind;
  pet.dataset.state = e.payload;
  playAnimation(e.payload);
});

// Qué avisa la alerta: por defecto un recordatorio; la batería usa otro icono.
listen("pet-alert-kind", (e) => {
  pet.dataset.alertKind = e.payload;
});

// Hacia dónde mira: la decide el backend (dirección del paseo o posición del cursor).
listen("pet-facing", (e) => {
  pet.dataset.facing = e.payload;
});
