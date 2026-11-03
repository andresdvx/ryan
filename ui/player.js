// Reproductor del chat: muestra lo que suena en Windows (Spotify, navegador, VLC…) y sus botones.
(() => {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;

  const root = document.getElementById("player");
  const art = document.getElementById("player-art");
  const titleEl = document.getElementById("player-title");
  const subEl = document.getElementById("player-sub");
  const bar = document.getElementById("player-bar");
  const fill = document.getElementById("player-fill");
  const elapsedEl = document.getElementById("player-elapsed");
  const totalEl = document.getElementById("player-total");
  const prevBtn = document.getElementById("player-prev");
  const toggleBtn = document.getElementById("player-toggle");
  const nextBtn = document.getElementById("player-next");

  const ICON_PLAY = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 5v14l11-7z"/></svg>';
  const ICON_PAUSE = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 5h4v14H6zm8 0h4v14h-4z"/></svg>';

  let media = null; // último estado recibido
  let receivedAt = 0; // performance.now() de ese estado
  let artworkKey = null;

  function formatTime(seconds) {
    const total = Math.max(0, Math.floor(seconds));
    const h = Math.floor(total / 3600);
    const m = Math.floor((total % 3600) / 60);
    const s = String(total % 60).padStart(2, "0");
    return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
  }

  // Posición estimada: la recibida más el tiempo que ha pasado si está sonando.
  function currentPosition() {
    if (!media) return 0;
    let position = media.position;
    if (media.state === "playing") position += (performance.now() - receivedAt) / 1000;
    return media.duration > 0 ? Math.min(position, media.duration) : position;
  }

  function paintProgress() {
    if (!media) return;
    const position = currentPosition();
    const known = media.duration > 0;
    fill.style.width = known ? `${(position / media.duration) * 100}%` : "0%";
    bar.dataset.live = known ? "false" : "true";
    elapsedEl.textContent = formatTime(position);
    totalEl.textContent = known ? formatTime(media.duration) : "";
  }

  function setArtwork(image) {
    if (image) {
      art.src = image;
      art.hidden = false;
    } else {
      art.removeAttribute("src");
      art.hidden = true;
    }
    root.classList.toggle("no-art", !image);
  }

  function render(state) {
    media = state;
    receivedAt = performance.now();
    if (!state) {
      root.hidden = true;
      artworkKey = null;
      return;
    }
    root.hidden = false;
    const playing = state.state === "playing";
    root.dataset.state = state.state;
    titleEl.textContent = state.title || "Sin título";
    titleEl.title = state.title || "";
    subEl.textContent = [state.artist, state.app].filter(Boolean).join(" · ");
    toggleBtn.innerHTML = playing ? ICON_PAUSE : ICON_PLAY;
    toggleBtn.title = toggleBtn.ariaLabel = playing ? "Pausar" : "Reproducir";
    toggleBtn.disabled = !state.can_play_pause;
    prevBtn.disabled = !state.can_previous;
    nextBtn.disabled = !state.can_next;
    if (state.artwork_key !== artworkKey) {
      // Pista nueva: se quita la carátula anterior hasta que llegue la nueva.
      artworkKey = state.artwork_key;
      setArtwork(null);
    }
    paintProgress();
  }

  async function press(action) {
    try {
      await invoke("media_control", { action });
    } catch (error) {
      subEl.textContent = String(error);
    }
    // Se refresca enseguida para que el botón refleje el cambio sin esperar al siguiente segundo.
    setTimeout(refresh, 250);
  }

  async function refresh() {
    try {
      const snapshot = await invoke("get_media");
      if (!snapshot) return render(null);
      render(snapshot.state);
      if (snapshot.artwork) setArtwork(snapshot.artwork);
    } catch {
      render(null);
    }
  }

  prevBtn.addEventListener("click", () => press("previous"));
  toggleBtn.addEventListener("click", () => press("play_pause"));
  nextBtn.addEventListener("click", () => press("next"));

  listen("media-state", (e) => render(e.payload));
  listen("media-artwork", (e) => {
    if (e.payload.key === artworkKey || artworkKey === null) {
      artworkKey = e.payload.key;
      setArtwork(e.payload.image);
    }
  });

  setInterval(() => {
    if (!root.hidden) paintProgress();
  }, 500);

  // El chat se oculta y se muestra sin recargarse: al volver, se pide el estado actual.
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) refresh();
  });
  window.addEventListener("focus", refresh);
  refresh();
})();
