// UI de productividad. El reloj y la persistencia viven en Rust, no en los temporizadores del WebView.
(() => {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  const $ = (id) => document.getElementById(id);
  let data = null;
  let focusBusy = false;
  let editingId = null;
  let reviewingId = null;
  let reviewingSnapshot = null;
  let apps = [];
  let saving = false;
  const labels = {
    open_application: "Abrir aplicación",
    open_website: "Abrir página web",
    play_music: "Reproducir en Spotify",
    set_reminder: "Crear recordatorio",
    list_tasks: "Consultar tareas",
    list_reminders: "Consultar recordatorios",
    system_info: "Consultar el equipo",
  };
  function message(id, text, error = false) {
    $(id).textContent = text;
    $(id).className = `message ${error ? "error" : "ok"}`;
  }
  function node(tag, text, className) {
    const el = document.createElement(tag);
    if (text) el.textContent = text;
    if (className) el.className = className;
    return el;
  }
  function action(label, callback, secondary = true) {
    const button = node("button", label, secondary ? "secondary" : "");
    button.type = "button";
    button.addEventListener("click", callback);
    return button;
  }
  function jump(id) {
    showTab("today");
    $(id).scrollIntoView({ block: "start", behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
    $(id).focus({ preventScroll: true });
  }
  $("jump-focus").addEventListener("click", () => jump("focus-heading"));
  $("jump-routines").addEventListener("click", () => jump("routines-heading"));

  function paintClock() {
    if (!data?.focus) return;
    const f = data.focus;
    const seconds = f.status === "running" ? Math.max(0, Math.ceil(f.deadline - Date.now() / 1000)) : f.remaining_seconds;
    $("focus-time").textContent = `${String(Math.floor(seconds / 60)).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}`;
    $("focus-clock").setAttribute("aria-label", `${f.phase === "focus" ? "Enfoque" : "Descanso"}: ${Math.floor(seconds / 60)} minutos y ${seconds % 60} segundos restantes`);
    const total = (f.phase === "focus" ? f.work_minutes : f.break_minutes) * 60;
    $("focus-clock").style.setProperty("--focus-progress", `${Math.min(100, Math.max(0, (1 - seconds / total) * 100))}%`);
  }
  function renderFocus() {
    if (!data?.focus) return;
    const f = data.focus;
    const active = f.status !== "ready";
    $("focus-phase").textContent = f.phase === "focus" ? "ENFOQUE" : "DESCANSO";
    $("focus-status").textContent = f.status === "running" ? "Tu tiempo, sin prisa." : f.status === "paused" ? "En pausa. Retoma cuando quieras." : f.phase === "break" ? "Buen trabajo. Es hora de una pausa." : "Un bloque para lo que importa.";
    $("focus-count").textContent = `${f.completed} bloque${f.completed === 1 ? "" : "s"} de enfoque completado${f.completed === 1 ? "" : "s"}.`;
    $("focus-start").hidden = f.status === "running";
    $("focus-start").textContent = f.status === "paused" ? "Reanudar" : f.phase === "break" ? "Iniciar descanso" : "Iniciar enfoque";
    $("focus-start").disabled = focusBusy;
    $("focus-pause").hidden = f.status !== "running";
    $("focus-pause").disabled = focusBusy;
    $("focus-stop").disabled = focusBusy || (!active && f.phase === "focus");
    for (const [id, value] of [["focus-work", f.work_minutes], ["focus-break", f.break_minutes]]) {
      $(id).disabled = active || focusBusy;
      if (active || document.activeElement !== $(id)) $(id).value = value;
    }
    paintClock();
  }
  async function refresh() {
    try {
      data = await invoke("get_productivity");
      renderFocus();
      renderRoutines();
    } catch (error) {
      $("focus-status").textContent = "No pude cargar el temporizador. Vuelve a abrir Hoy para reintentar.";
      message("routine-message", String(error), true);
    }
  }
  async function focusAction(kind) {
    if (!data || focusBusy) return;
    const work = Number($("focus-work").value), rest = Number($("focus-break").value);
    if (!Number.isInteger(work) || work < 1 || work > 180 || !Number.isInteger(rest) || rest < 1 || rest > 60) {
      return message("focus-message", "Enfoque: 1–180 min. Descanso: 1–60 min.", true);
    }
    focusBusy = true;
    renderFocus();
    try {
      data.focus = await invoke("focus_action", { action: kind, workMinutes: work, breakMinutes: rest });
      message("focus-message", kind === "stop" ? "Temporizador finalizado. Tus modos no han cambiado." : "");
    } catch (error) { message("focus-message", String(error), true); }
    finally { focusBusy = false; await refresh(); }
  }
  $("focus-start").addEventListener("click", () => focusAction(data?.focus.status === "paused" ? "resume" : "start"));
  $("focus-pause").addEventListener("click", () => focusAction("pause"));
  $("focus-stop").addEventListener("click", () => focusAction("stop"));
  $("focus-quiet").addEventListener("click", async () => {
    try {
      await invoke("set_profile", { id: "focus" });
      message("focus-message", "Modo Concentración activo. Vuelve a Normal desde Modo cuando quieras.");
    } catch (error) { message("focus-message", String(error), true); }
  });

  function describe(step) {
    const a = step.args;
    switch (step.tool) {
      case "open_application": return `Abrir ${apps.find(app => app.key === a.application)?.name || a.application}`;
      case "open_website": return `Abrir ${a.url}`;
      case "play_music": return `Reproducir «${a.query}» en Spotify`;
      case "set_reminder": return `Recordar «${a.text}» en ${a.delay_seconds / 60} min`;
      default: return labels[step.tool] || step.tool;
    }
  }
  function stepList(steps) {
    const list = node("ol");
    for (const step of steps) list.append(node("li", describe(step)));
    return list;
  }
  function renderRoutines() {
    const active = data?.active_routine;
    $("routine-running").hidden = !active;
    if (active) $("routine-progress").textContent = `En curso: ${active.name}. Revisa el chat si necesita tu permiso.`;
    $("routine-new").disabled = Boolean(active) || saving;
    $("routine-confirm").disabled = Boolean(active);
    $("routines-empty").hidden = Boolean(data?.routines?.length);
    const cards = (data?.routines || []).map(routine => {
      const card = node("article", "", "routine-card");
      const title = node("strong", routine.name);
      const controls = node("div", "", "row");
      const review = action("Revisar y ejecutar", () => reviewRoutine(routine), false);
      const edit = action("Editar", () => editRoutine(routine));
      const remove = action("Eliminar", async () => {
        if (remove.dataset.armed !== "true") {
          remove.dataset.armed = "true";
          remove.textContent = "Confirmar eliminación";
          setTimeout(() => { if (remove.isConnected) { delete remove.dataset.armed; remove.textContent = "Eliminar"; } }, 5000);
          return;
        }
        try { await invoke("delete_routine", { id: routine.id }); await refresh(); }
        catch (error) { message("routine-message", String(error), true); }
      });
      for (const button of [review, edit, remove]) button.disabled = Boolean(active);
      controls.append(review, edit, remove);
      card.append(title, stepList(routine.steps), controls);
      return card;
    });
    $("routine-list").replaceChildren(...cards);
  }
  function reviewRoutine(routine) {
    reviewingId = routine.id;
    reviewingSnapshot = structuredClone({ name: routine.name, steps: routine.steps });
    $("routine-form").hidden = true;
    $("routine-review-title").textContent = `Ejecutar «${routine.name}»`;
    $("routine-review-steps").replaceChildren(...stepList(routine.steps).childNodes);
    $("routine-review").hidden = false;
    $("routine-confirm").focus();
  }
  $("routine-review-cancel").addEventListener("click", () => {
    $("routine-review").hidden = true;
    reviewingId = null;
    reviewingSnapshot = null;
    $("routine-new").focus();
  });
  $("routine-confirm").addEventListener("click", async () => {
    if (reviewingId === null) return;
    $("routine-confirm").disabled = true;
    try {
      await invoke("run_routine", { id: reviewingId, expected: reviewingSnapshot });
      $("routine-review").hidden = true;
      message("routine-message", "Rutina iniciada. Puedes seguir sus resultados en el chat.");
      await refresh();
    } catch (error) { message("routine-message", String(error), true); }
    finally { $("routine-confirm").disabled = Boolean(data?.active_routine); }
  });
  $("routine-stop").addEventListener("click", async () => {
    try {
      await invoke("cancel_routine");
      message("routine-message", "Se detendrá antes del siguiente paso. Si hay una confirmación abierta, puedes cancelarla en el chat.");
    } catch (error) { message("routine-message", String(error), true); }
  });

  async function loadApps() {
    const catalog = await invoke("list_apps", { refresh: false });
    apps = [...catalog.builtin.filter(a => a.enabled && a.installed), ...catalog.apps.filter(a => a.enabled && a.key)];
  }
  function labeledControl(row, label, control) {
    const wrapper = node("label", label);
    wrapper.append(control);
    row.append(wrapper);
    return control;
  }
  function input(type, value, placeholder, max) {
    const el = node("input");
    el.type = type;
    el.value = value ?? "";
    el.placeholder = placeholder || "";
    el.required = true;
    if (max) el.maxLength = max;
    return el;
  }
  function addStep(step = { tool: "list_tasks", args: {} }) {
    if ($("routine-steps").children.length >= 8) return;
    const row = node("div", "", "routine-step");
    const heading = node("div", "", "routine-step-heading");
    const select = node("select");
    select.setAttribute("aria-label", "Acción del paso");
    for (const [key, label] of Object.entries(labels)) select.add(new Option(label, key));
    select.value = step.tool;
    const fields = node("div", "", "routine-step-fields");
    function renderFields(args = {}) {
      fields.replaceChildren();
      if (select.value === "open_application") {
        const app = node("select");
        app.className = "step-app";
        app.required = true;
        app.add(new Option("Elige una aplicación permitida", ""));
        for (const entry of apps) app.add(new Option(entry.name, entry.key));
        if (args.application && !apps.some(a => a.key === args.application)) app.add(new Option(`${args.application} (revisa sus permisos)`, args.application));
        app.value = args.application || "";
        labeledControl(fields, "Aplicación", app);
        fields.append(node("p", "Solo aparecen las aplicaciones permitidas. Puedes cambiarlas en Ajustes → Sistema.", "hint"));
      } else if (select.value === "open_website") {
        const url = input("url", args.url, "https://…", 2048);
        url.className = "step-url";
        labeledControl(fields, "Dirección https", url);
      } else if (select.value === "play_music") {
        const query = input("text", args.query, "Canción o artista", 100);
        query.className = "step-query";
        labeledControl(fields, "Qué reproducir en Spotify", query);
      } else if (select.value === "set_reminder") {
        const text = input("text", args.text, "Qué recordar", 200);
        text.className = "step-text";
        labeledControl(fields, "Recordatorio", text);
        const minutes = input("number", args.delay_seconds ? args.delay_seconds / 60 : 20);
        minutes.min = 1; minutes.max = 10080; minutes.className = "step-minutes";
        labeledControl(fields, "Dentro de (min)", minutes);
      }
    }
    select.addEventListener("change", () => renderFields());
    const up = action("↑", () => { if (row.previousElementSibling) row.parentNode.insertBefore(row, row.previousElementSibling); });
    up.setAttribute("aria-label", "Subir paso");
    const down = action("↓", () => { if (row.nextElementSibling) row.parentNode.insertBefore(row.nextElementSibling, row); });
    down.setAttribute("aria-label", "Bajar paso");
    const remove = action("Quitar", () => { row.remove(); updateAddStep(); });
    heading.append(select, up, down, remove);
    row.append(heading, fields);
    $("routine-steps").append(row);
    renderFields(step.args);
    updateAddStep();
  }
  function updateAddStep() { $("routine-add-step").disabled = $("routine-steps").children.length >= 8; }
  async function editRoutine(routine = null) {
    try { await loadApps(); }
    catch { apps = []; }
    editingId = routine?.id ?? null;
    $("routine-name").value = routine?.name || "";
    $("routine-steps").replaceChildren();
    for (const step of routine?.steps || [{ tool: "list_tasks", args: {} }]) addStep(step);
    $("routine-review").hidden = true;
    $("routine-form").hidden = false;
    message("routine-message", "");
    $("routine-name").focus();
  }
  $("routine-new").addEventListener("click", () => editRoutine());
  $("routine-add-step").addEventListener("click", () => addStep());
  $("routine-edit-cancel").addEventListener("click", () => { $("routine-form").hidden = true; $("routine-new").focus(); });
  $("routine-form").addEventListener("submit", async event => {
    event.preventDefault();
    if (saving) return;
    const steps = [...$("routine-steps").children].map(row => {
      const tool = row.querySelector("select").value;
      let args = {};
      if (tool === "open_application") args = { application: row.querySelector(".step-app").value };
      if (tool === "open_website") args = { url: row.querySelector(".step-url").value.trim() };
      if (tool === "play_music") args = { query: row.querySelector(".step-query").value.trim(), service: "spotify" };
      if (tool === "set_reminder") args = { text: row.querySelector(".step-text").value.trim(), delay_seconds: Number(row.querySelector(".step-minutes").value) * 60 };
      return { tool, args };
    });
    saving = true;
    $("routine-save").disabled = true;
    try {
      await invoke("save_routine", { id: editingId, name: $("routine-name").value.trim(), steps });
      $("routine-form").hidden = true;
      message("routine-message", "Rutina guardada. No se ejecuta hasta que revises y confirmes sus pasos.");
      await refresh();
      $("routine-new").focus();
    } catch (error) { message("routine-message", String(error), true); }
    finally { saving = false; $("routine-save").disabled = false; $("routine-new").disabled = Boolean(data?.active_routine); }
  });
  listen("productivity-changed", refresh);
  listen("routine-finished", event => message("routine-message", event.payload.message, !event.payload.ok));
  listen("task-progress", event => {
    if (data?.active_routine) $("routine-progress").textContent = `${data.active_routine.name} · paso ${event.payload.step}/${event.payload.total}: ${labels[event.payload.tool] || event.payload.tool}`;
  });
  document.querySelector('[data-tab="today"]').addEventListener("click", refresh);
  window.addEventListener("focus", refresh);
  document.addEventListener("visibilitychange", () => { if (!document.hidden) refresh(); });
  setInterval(() => { if (!$("view-today").hidden && !document.hidden) paintClock(); }, 1000);
  refresh();
})();
