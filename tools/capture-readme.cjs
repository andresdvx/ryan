// Ejecutar con Playwright disponible en NODE_PATH; instrucciones en docs/screenshots.md.
const { chromium } = require("playwright");
const fs = require("node:fs/promises");
const path = require("node:path");
const http = require("node:http");
const fixtures = require("./docs-fixtures.cjs");
const root = path.resolve(__dirname, "../ui");
const output = path.resolve(__dirname, "../docs/images");
const types = { ".html": "text/html", ".css": "text/css", ".js": "text/javascript", ".json": "application/json", ".png": "image/png" };

async function main() {
  // Sirve únicamente la UI; la API nativa se sustituye antes de cargar sus scripts.
  const server = http.createServer(async (req, res) => {
    try {
      const filename = path.resolve(root, "." + new URL(req.url, "http://localhost").pathname);
      if (!filename.startsWith(root + path.sep)) throw new Error("Ruta no permitida");
      let data;
      try {
        data = await fs.readFile(filename);
      } catch (error) {
        // ui/assets/ se genera al compilar; permite capturar también desde un clon limpio.
        const assetRoot = path.join(root, "assets") + path.sep;
        if (!filename.startsWith(assetRoot)) throw error;
        data = await fs.readFile(path.resolve(__dirname, "../assets", filename.slice(assetRoot.length)));
      }
      if (filename === path.join(root, "chat.js")) {
        data = Buffer.concat([data, Buffer.from("\nwindow.docs = { showTab, showPanel, addMessage, addUserMessage, hideIdeas, loadSettings, openLook, loadTasks, loadReminders };\n")]);
      }
      res.setHeader("Content-Type", types[path.extname(filename)] || "application/octet-stream");
      res.end(data);
    } catch {
      res.writeHead(404).end();
    }
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  let browser;
  try {
    browser = await chromium.launch({ channel: process.env.DOCS_BROWSER || "msedge", headless: true });
    const context = await browser.newContext({ viewport: { width: 880, height: 720 }, deviceScaleFactor: 1.5, locale: "es-ES", timezoneId: "Europe/Madrid", reducedMotion: "reduce" });
    await context.addInitScript(({ data }) => {
      window.demo = data;
      window.demoEvents = {};
      const listen = async (name, fn) => {
        (window.demoEvents[name] ??= []).push(fn);
        return () => {};
      };
      window.emitDemo = (name, payload) => (window.demoEvents[name] || []).forEach(fn => fn({ payload }));
      const invoke = async (cmd, args = {}) => {
        switch (cmd) {
          case "get_settings": return data.settings;
          case "get_productivity": return data.productivity;
          case "get_character": return data.settings.character;
          case "get_assistant_name": return data.settings.assistant_name;
          case "list_reminders": return { items: data.reminders, max: 50 };
          case "list_tasks": return { items: data.tasks };
          case "list_conversations": return data.conversations;
          case "list_activity": return data.activity;
          case "list_memories": return [{ id: 1, text: "Prefieres respuestas breves y música tranquila para trabajar." }];
          case "list_ollama_models": return ["qwen3.5:9b", "qwen3:8b", "llama3.2:3b"];
          case "get_media": return { state: data.media, artwork: null };
          case "get_system_status": return { battery: { percent: 82, charging: false }, memory_used_percent: 42, memory_total_bytes: 16 * 2 ** 30, disk_total_bytes: 512 * 2 ** 30, disk_free_bytes: 240 * 2 ** 30, uptime_seconds: 7800 };
          case "list_capturable_windows": return [{ title: "Notas del proyecto — Bloc de notas", process: "notepad.exe" }];
          case "capture_window_preview": return window.demoCapture;
          case "resize_quick": return;
          case "quick_submit": return;
          case "hide_quick_cmd": return;
          case "list_apps": return { builtin: [], apps: [] };
          case "file_menu_state": return data.fileMenu;
          case "file_menu_resize": return;
          case "file_menu_convert": return { outputs: ["foto-proyecto.jpg"], dir: "Escritorio", notes: ["El archivo original se conserva."], failures: [] };
          default: throw new Error(`Capturas: comando sin simular (${cmd})`);
        }
      };
      window.__TAURI__ = { core: { invoke }, event: { listen }, window: { getCurrentWindow: () => ({ onFocusChanged: async () => () => {} }) } };
    }, { data: fixtures });
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.clock.setFixedTime(new Date(fixtures.now * 1000));
    const base = `http://127.0.0.1:${server.address().port}`;
    async function load() {
      await page.setViewportSize({ width: 880, height: 720 });
      await page.goto(`${base}/chat.html`);
      await page.waitForFunction(() => window.docs && document.getElementById("model-pill").hidden === false);
      await page.locator("#header-avatar").waitFor({ state: "visible" });
      await page.evaluate(async () => {
        docs.hideIdeas();
        document.getElementById("history").replaceChildren();
        document.getElementById("player").hidden = true;
        await document.fonts.ready;
      });
    }
    async function save(name, locator) {
      await page.evaluate(async () => {
        await Promise.all([...document.images].filter(image => image.getClientRects().length && image.getAttribute("src")).map(image => image.decode()));
      });
      await page.evaluate(() => document.activeElement?.blur());
      // Espera al dibujo sin depender de pausas arbitrarias.
      await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
      if (locator) await locator.screenshot({ path: path.join(output, `${name}.png`), animations: "disabled" });
      else await page.screenshot({ path: path.join(output, `${name}.png`), animations: "disabled" });
      console.log(`✓ ${name}.png`);
    }
    async function scrollPanelTo(selector) {
      await page.locator(selector).evaluate(el => {
        const panel = el.closest(".panels");
        panel.scrollTop += el.getBoundingClientRect().top - panel.getBoundingClientRect().top - 20;
      });
    }
    await fs.mkdir(output, { recursive: true });
    await load();
    await page.evaluate(() => {
      docs.addMessage("assistant", "¡Buenos días! ¿Qué hacemos hoy?");
      docs.addMessage("user", "Ayúdame a organizar mi día: ¿qué tengo pendiente?");
      docs.addMessage("assistant", "Tienes **2 tareas pendientes**:\n\n- Preparar la propuesta del proyecto. Falta revisar el presupuesto.\n- Ordenar los documentos de la semana.\n\nTu próximo recordatorio es a las **10:20**: estirar las piernas y beber agua.\n\n¿Empezamos por la propuesta?");
    });
    await save("chat");
    await page.setViewportSize({ width: 620, height: 720 });
    await save("chat-compacto");

    await load();
    await page.evaluate(() => {
      docs.addMessage("user", "Reproduce (canción) en Spotify");
      emitDemo("chat-tool", { ...demo.activity[2], at: demo.now });
      docs.addMessage("assistant", "Listo. Puedes pausar, cambiar de pista o seguir escuchando mientras trabajas.");
      document.getElementById("player").hidden = false;
      emitDemo("media-state", { ...demo.media, state: "playing" });
    });
    await save("musica");

    await load();
    await page.evaluate(() => {
      docs.addMessage("user", "Analiza informe-proyecto.pdf");
      docs.addMessage("assistant", "## Análisis de informe-proyecto.pdf\n\nEl documento describe el avance del proyecto y sus próximos hitos.\n\n**Lo más importante**\n- El diseño está aprobado y la implementación ha comenzado.\n- Falta validar el presupuesto antes de cerrar la propuesta.\n- La siguiente revisión está prevista para el jueves.\n\n**Punto a revisar**\nEl calendario depende de esa validación. Conviene confirmarla antes de comprometer la fecha de entrega.");
    });
    await save("analizar-archivo");

    await load();
    await page.evaluate(() => {
      docs.addMessage("user", "Resume notas-reunion.docx");
      docs.addMessage("assistant", "## Resumen de notas-reunion.docx\n\nEl equipo acordó preparar la propuesta y revisar el presupuesto antes del jueves.\n\n**Próximos pasos**\n1. Completar el borrador de la propuesta.\n2. Confirmar el presupuesto con el equipo.\n3. Compartir un resumen antes de la siguiente reunión.\n\n**Pendiente de decidir:** la fecha final de entrega.");
    });
    await save("resumir-archivo");

    await load();
    await page.evaluate(() => {
      docs.addMessage("user", "Organiza mi Escritorio en carpetas según el tipo de archivo.");
      emitDemo("chat-tool", {
        id: "organize-demo", tool: "organize_folder", text: "✔ Escritorio organizado: 18 archivos en 4 carpetas.",
        summary: "18 archivos agrupados en Documentos, Imágenes, Comprimidos y Otros.",
        args: { folder: "C:\\Users\\Demo\\Desktop" }, data: { folder: "C:\\Users\\Demo\\Desktop", moved: 18 },
        at: demo.now, duration_ms: 185, ok: true, confirmed: true, undo: "restaurar la organización anterior",
      });
      docs.addMessage("assistant", "Tu Escritorio ya está ordenado:\n\n- **Documentos:** 8 archivos.\n- **Imágenes:** 6 archivos.\n- **Comprimidos:** 2 archivos.\n- **Otros:** 2 archivos.\n\nNo borré nada. Puedes revisar el detalle o solicitar deshacer la organización desde Registro.");
    });
    await save("organizar-escritorio");

    await load();
    await page.evaluate(() => {
      docs.addMessage("user", "Crea una carpeta Proyecto en Documentos y mueve allí propuesta.pdf de Descargas.");
      emitDemo("chat-tool", { id: "folder-demo", tool: "create_folder", text: "✔ Creé la carpeta Documentos\\Proyecto.", summary: "Carpeta Proyecto creada.", args: { path: "C:\\Users\\Demo\\Documents\\Proyecto" }, data: { path: "C:\\Users\\Demo\\Documents\\Proyecto" }, at: demo.now, duration_ms: 18, ok: true, confirmed: true, undo: "quitar la carpeta creada" });
      emitDemo("chat-tool", { ...demo.activity[0], at: demo.now });
      docs.addMessage("assistant", "Listo. La propuesta está en **Documentos → Proyecto**.\n\nPuedes mostrar el archivo en el Explorador desde su detalle. Si necesitas revertirlo, solicita deshacer desde Registro.");
    });
    await save("organizar-carpetas");

    await load();
    await page.evaluate(() => {
      docs.addMessage("user", "Elimina la carpeta de borradores antiguos.");
      emitDemo("confirmation-request", { id: "demo-confirm", tool: "delete_file", risk: "dangerous", detailed: true, preview: "Enviar a la Papelera:\nC:\\Users\\Demo\\Documents\\Borradores antiguos\n\nIncluye 8 archivos y 2 subcarpetas.\nPodrás recuperarlos desde la Papelera de Windows." });
    });
    await save("confirm");

    for (const [filename, panel] of [["ai", "ai"], ["pet", "pet"], ["security", "security"], ["editor-herramientas", "security"]]) {
      await load();
      await page.evaluate(async panel => { docs.showTab("settings"); await docs.loadSettings(); docs.showPanel(panel); }, panel);
      if (filename === "editor-herramientas") {
        await page.locator(".tool-group").first().evaluate(el => { el.open = true; });
        await scrollPanelTo("#tools-search");
      }
      if (filename === "pet") await scrollPanelTo("#name-form");
      await save(filename);
    }
    await load();
    await page.evaluate(async () => { docs.showTab("reminders"); await docs.loadTasks(); await docs.loadReminders(); });
    await save("tareas");
    await page.locator("#reminder-repeat").selectOption("weekly");
    await page.locator("#reminder-weekday").selectOption("5");
    await page.locator("#reminder-clock").fill("09:00");
    await page.locator("#reminder-text").fill("Revisar el proyecto");
    await page.locator("#reminder-form").evaluate(el => el.closest(".panels").scrollTop = el.offsetTop - el.closest(".panels").offsetTop - 14);
    await save("recordatorios");

    await load();
    await page.evaluate(() => docs.addMessage("assistant", "Puedes cambiar de IA sin salir de aquí. Elige un proveedor o un modelo local."));
    await page.locator("#model-pill").click();
    await page.locator(".menu-models .menu-item").first().waitFor();
    await save("modelo");

    await load();
    await page.evaluate(() => docs.addMessage("assistant", "Escribe / para ver tus comandos disponibles."));
    await page.locator("#input").fill("/");
    await page.locator("#palette").waitFor({ state: "visible" });
    await save("comandos");

    await load();
    await page.evaluate(() => docs.showTab("history"));
    await page.locator(".conv").first().waitFor();
    await save("historial");

    await load();
    await page.evaluate(async () => { docs.showTab("today"); await docs.loadReminders(); });
    await page.locator(".meter").first().waitFor();
    await save("hoy");
    await page.evaluate(() => {
      demo.productivity.focus.status = "running";
      demo.productivity.focus.deadline = demo.now + 1013;
      emitDemo("productivity-changed");
    });
    await page.locator("#focus-pause").waitFor({ state: "visible" });
    await scrollPanelTo(".focus-card");
    await save("enfoque");
    await page.evaluate(() => {
      demo.productivity.focus.status = "ready";
      demo.productivity.focus.deadline = null;
      emitDemo("productivity-changed");
    });
    await page.locator(".routines-card .routine-card button").first().click();
    await scrollPanelTo(".routines-card");
    await save("rutinas");
    await page.locator("#routine-review-cancel").click();
    await scrollPanelTo(".today-system");
    await save("hoy-equipo");
    await page.locator("#view-today .panels").evaluate(el => { el.scrollTop = 0; });
    await page.evaluate(async () => { demo.settings.profile = "work"; await docs.loadSettings(); });
    await scrollPanelTo(".today-mode");
    await save("modos");

    await load();
    await page.evaluate(() => {
      docs.addMessage("user", "Mueve propuesta.pdf a la carpeta del proyecto.");
      emitDemo("chat-tool", demo.activity[0]);
    });
    await page.locator(".detail-toggle").click();
    await save("detalle-herramienta");

    await load();
    await page.evaluate(() => docs.showTab("activity"));
    await page.locator(".act").first().waitFor();
    // El registro es más largo que una ventana: se captura su inicio, sin alterar su interfaz.
    await save("actividad");

    // Documento ficticio para la previsualización; no se captura ninguna ventana del usuario.
    const preview = await context.newPage();
    await preview.setViewportSize({ width: 720, height: 360 });
    await preview.setContent('<html lang="es"><body style="margin:0;background:#f7f7f7;color:#252525;font:18px/1.7 Segoe UI,sans-serif"><div style="background:#e9e9e9;padding:10px 24px;font-size:13px">Notas del proyecto — Bloc de notas · Datos de ejemplo</div><main style="padding:22px 32px"><h2 style="margin:0 0 12px">Reunión del jueves</h2>• Revisar la propuesta y el presupuesto.<br>• Confirmar los próximos pasos.<br>• Compartir el resumen con el equipo.</main></body></html>');
    const image = await preview.screenshot();
    await preview.close();
    await load();
    await page.evaluate(async data => {
      docs.addMessage("user", "Mira mis notas y ayúdame a preparar la reunión.");
      window.demoCapture = { title: "Notas del proyecto — Bloc de notas", process: "notepad.exe", mime: "image/png", data, width: 720, height: 360 };
      await docs.openLook();
    }, image.toString("base64"));
    await page.locator("#look-thumb").waitFor({ state: "visible" });
    await save("mirar-ventana");
    await load();
    await page.evaluate(data => {
      docs.addUserMessage("Explícame esta ventana y dime qué tengo que hacer.", [{ mime: "image/png", data, label: "Notas del proyecto — ejemplo" }]);
      docs.addMessage("assistant", "Veo unas notas para la **reunión del jueves**.\n\nTienes tres puntos por preparar:\n1. Revisar la propuesta y el presupuesto.\n2. Confirmar los próximos pasos.\n3. Compartir el resumen con el equipo.\n\nPuedes empezar por comprobar que el presupuesto está actualizado y llevar una lista breve de decisiones pendientes.");
    }, image.toString("base64"));
    await save("explicar-pantalla");

    await page.setViewportSize({ width: 340, height: 176 });
    await page.goto(`${base}/dropbar.html`);
    await page.waitForFunction(() => window.demoEvents["dropbar-open"]);
    await page.evaluate(() => emitDemo("dropbar-open"));
    await page.waitForFunction(() => document.documentElement.style.getPropertyValue("--accent-rgb"));
    await save("arrastrar-archivos");

    await page.setViewportSize({ width: 380, height: 420 });
    await page.goto(`${base}/filemenu.html`);
    await page.locator('#fm-convert [data-id="jpg"]').waitFor();
    await page.waitForFunction(() => document.documentElement.style.getPropertyValue("--accent-rgb"));
    await save("convertir-archivo", page.locator("#menu"));
    await page.locator('#fm-convert [data-id="jpg"]').click();
    await page.locator("#fm-done").waitFor({ state: "visible" });
    await save("conversion-lista", page.locator("#menu"));

    await page.setViewportSize({ width: 640, height: 560 });
    await page.goto(`${base}/quick.html`);
    await page.waitForFunction(() => window.demoEvents["quick-opened"]);
    await page.evaluate(() => emitDemo("quick-opened"));
    await page.evaluate(() => {
      document.getElementById("q-input").value = "¿Qué tengo pendiente hoy?";
      document.getElementById("q-input").dispatchEvent(new Event("input"));
    });
    // Se usa el formulario real; quick_submit no ejecuta ninguna acción nativa.
    await page.locator("#q-form").evaluate(el => el.requestSubmit());
    await page.evaluate(() => {
      emitDemo("chat-final", "Tienes **2 tareas pendientes** y 3 recordatorios.\n\n- Preparar la propuesta: falta revisar el presupuesto.\n- Ordenar los documentos de la semana.\n\nTu próxima pausa es a las **10:20**.");
      emitDemo("character-state", "idle");
    });
    await page.locator("#q-foot").waitFor({ state: "visible" });
    await save("barra-rapida", page.locator("#bar"));
    if (errors.length) throw new Error(errors.join("\n"));
    await context.close();
  } finally {
    await browser?.close();
    await new Promise(resolve => server.close(resolve));
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
