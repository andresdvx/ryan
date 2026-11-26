// Prueba la UI real con una API nativa simulada. La lógica Rust tiene sus propias pruebas.
const { chromium } = require("playwright");
const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const path = require("node:path");
const http = require("node:http");
const fixtures = require("./docs-fixtures.cjs");
const root = path.resolve(__dirname, "../ui");

async function main() {
  const server = http.createServer(async (req, res) => {
    try {
      const name = path.resolve(root, "." + new URL(req.url, "http://localhost").pathname);
      if (!name.startsWith(root + path.sep)) throw new Error("Ruta no permitida");
      let body;
      try { body = await fs.readFile(name); }
      catch (error) {
        const prefix = path.join(root, "assets") + path.sep;
        if (!name.startsWith(prefix)) throw error;
        body = await fs.readFile(path.resolve(__dirname, "../assets", name.slice(prefix.length)));
      }
      res.setHeader("Content-Type", ({ ".js": "text/javascript", ".css": "text/css", ".html": "text/html", ".json": "application/json", ".png": "image/png" })[path.extname(name)] || "application/octet-stream");
      res.end(body);
    } catch { res.writeHead(404).end(); }
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  let browser;
  try {
    browser = await chromium.launch({ channel: process.env.DOCS_BROWSER || "msedge", headless: true });
    const context = await browser.newContext({ viewport: { width: 880, height: 720 }, reducedMotion: "reduce" });
    await context.addInitScript(data => {
      const listeners = {};
      window.calls = [];
      window.emitTest = (name, payload) => (listeners[name] || []).forEach(fn => fn({ payload }));
      const invoke = async (cmd, args = {}) => {
        window.calls.push({ cmd, args });
        const p = data.productivity;
        switch (cmd) {
          case "get_settings": return data.settings;
          case "get_character": return data.settings.character;
          case "get_assistant_name": return data.settings.assistant_name;
          case "get_productivity": return structuredClone(p);
          case "get_media": return null;
          case "list_memories": return [];
          case "list_tasks": return { items: data.tasks };
          case "list_reminders": return { items: data.reminders, max: 50 };
          case "list_conversations": return data.conversations;
          case "list_apps": return { builtin: [{ key: "notepad", name: "Bloc de notas", enabled: true, installed: true }], apps: [] };
          case "get_system_status": return { memory_used_percent: 30, memory_total_bytes: 16 * 2 ** 30, disk_total_bytes: 0, uptime_seconds: 1 };
          case "add_reminder": return "mañana a las 09:00";
          case "cancel_reminder": return true;
          case "focus_action":
            if (args.action === "start") { p.focus.status = "running"; p.focus.work_minutes = args.workMinutes; p.focus.break_minutes = args.breakMinutes; p.focus.deadline = Date.now() / 1000 + args.workMinutes * 60; }
            if (args.action === "pause") { p.focus.status = "paused"; p.focus.remaining_seconds = 100; p.focus.deadline = null; }
            if (args.action === "resume") { p.focus.status = "running"; p.focus.deadline = Date.now() / 1000 + p.focus.remaining_seconds; }
            if (args.action === "stop") { p.focus.status = "ready"; p.focus.phase = "focus"; p.focus.deadline = null; p.focus.remaining_seconds = 1500; }
            return structuredClone(p.focus);
          case "save_routine": {
            const routine = { ...args, id: args.id || 2 };
            p.routines = p.routines.filter(r => r.id !== routine.id).concat(routine);
            return routine;
          }
          case "delete_routine": p.routines = p.routines.filter(r => r.id !== args.id); return;
          case "run_routine": p.active_routine = { id: args.id, name: p.routines.find(r => r.id === args.id).name }; return;
          case "cancel_routine": p.active_routine = null; window.emitTest("productivity-changed"); return;
          case "set_profile": data.settings.profile = args.id; window.emitTest("profile-changed"); return;
          default: throw new Error(`Falta simular ${cmd}`);
        }
      };
      window.__TAURI__ = { core: { invoke }, event: { listen: async (name, fn) => { (listeners[name] ||= []).push(fn); return () => {}; } }, window: { getCurrentWindow: () => ({ onFocusChanged: async () => () => {} }) } };
    }, fixtures);
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", e => errors.push(e.message));
    await page.goto(`http://127.0.0.1:${server.address().port}/chat.html`);
    await page.locator("#model-pill").waitFor({ state: "visible" });
    await page.locator('[data-tab="reminders"]').click();
    await page.locator("#reminder-repeat").selectOption("weekly");
    assert.equal(await page.locator("#reminder-mode").inputValue(), "at");
    assert.equal(await page.locator("#reminder-mode").isDisabled(), true);
    await page.locator("#reminder-weekday").selectOption("5");
    await page.locator("#reminder-clock").fill("09:00");
    await page.locator("#reminder-text").fill("Revisar el proyecto");
    await page.locator("#reminder-add").click();
    await page.locator("#reminder-message").filter({ hasText: "Listo" }).waitFor();
    const weekly = await page.evaluate(() => window.calls.find(c => c.cmd === "add_reminder").args);
    assert.deepEqual(weekly.recurrence, { kind: "weekly", weekday: 5 });
    assert.equal(weekly.at, "09:00");
    await page.locator("#reminder-repeat").selectOption("interval");
    await page.locator("#reminder-minutes").fill("120");
    await page.locator("#reminder-text").fill("Agua");
    await page.locator("#reminder-add").click();
    await page.waitForFunction(() => window.calls.filter(c => c.cmd === "add_reminder").length === 2);
    const interval = await page.evaluate(() => window.calls.filter(c => c.cmd === "add_reminder").at(-1).args);
    assert.deepEqual(interval.recurrence, { kind: "interval", seconds: 7200 });

    await page.locator('[data-tab="today"]').click();
    await page.locator("#focus-start").click();
    await page.locator("#focus-pause").waitFor({ state: "visible" });
    await page.locator("#focus-pause").click();
    await page.locator("#focus-start").filter({ hasText: "Reanudar" }).waitFor();
    await page.locator("#focus-start").click();
    await page.locator("#focus-stop").click();
    assert.equal(await page.locator("#focus-time").textContent(), "25:00");

    await page.locator("#routine-new").click();
    await page.locator("#routine-name").fill("Rutina de prueba");
    await page.locator("#routine-add-step").click();
    await page.locator(".routine-step-heading select").nth(1).selectOption("open_website");
    await page.locator(".step-url").fill("https://example.com/");
    await page.locator("#routine-save").click();
    await page.locator(".routine-card").filter({ hasText: "Rutina de prueba" }).waitFor();
    assert.equal(await page.evaluate(() => window.calls.filter(c => c.cmd === "run_routine").length), 0, "guardar no ejecuta");
    await page.locator(".routine-card").filter({ hasText: "Rutina de prueba" }).getByRole("button", { name: "Revisar y ejecutar" }).click();
    assert.equal(await page.evaluate(() => window.calls.filter(c => c.cmd === "run_routine").length), 0, "revisar no ejecuta");
    await page.locator("#routine-confirm").click();
    await page.locator("#routine-running").waitFor({ state: "visible" });
    assert.equal(await page.evaluate(() => window.calls.find(c => c.cmd === "run_routine").args.expected.steps.length), 2, "se envían los pasos revisados para detectar cambios");
    await page.locator("#routine-stop").click();
    await page.locator("#routine-running").waitFor({ state: "hidden" });
    const card = page.locator(".routine-card").filter({ hasText: "Rutina de prueba" });
    await card.getByRole("button", { name: "Editar", exact: true }).click();
    assert.equal(await page.locator("#routine-steps .routine-step").count(), 2);
    await page.locator(".routine-step-heading button[aria-label='Subir paso']").nth(1).click();
    assert.equal(await page.locator(".routine-step-heading select").first().inputValue(), "open_website");
    await page.locator("#routine-edit-cancel").click();
    await card.getByRole("button", { name: "Eliminar", exact: true }).click();
    await card.getByRole("button", { name: "Confirmar eliminación" }).click();
    await card.waitFor({ state: "detached" });
    for (const width of [380, 620, 880]) {
      await page.setViewportSize({ width, height: 720 });
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, `sin desbordamiento a ${width}px`);
      assert.equal(await page.locator("#view-today").evaluate(el => el.scrollWidth > el.clientWidth), false);
    }
    assert.deepEqual(errors, []);
    console.log("✓ Recurrencia, temporizador, edición/revisión/ejecución/cancelación de rutinas y diseños 380/620/880 px.");
    await context.close();
  } finally {
    await browser?.close();
    await new Promise(resolve => server.close(resolve));
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
