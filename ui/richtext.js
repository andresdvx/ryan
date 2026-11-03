// Texto enriquecido del chat: un subconjunto pequeño de Markdown (negrita, cursiva, `código`, bloques de código,
// listas, títulos y enlaces https). Todo se construye con nodos DOM —nunca con innerHTML—, así que un texto
// de la IA o de una página web no puede inyectar HTML ni scripts.
(() => {
  const INLINE =
    /`([^`\n]+)`|\*\*(?=\S)([^*\n]+?)(?<=\S)\*\*|(?<![\w*])\*(?!\s)([^*\n]+?)(?<!\s)\*(?![\w*])|(https:\/\/[^\s<>"')\]]+)/g;

  // `onLink(url)` se llama al pulsar un enlace; la app valida cada dirección antes de abrirla.
  function appendInline(parent, text, onLink) {
    let last = 0;
    for (const match of text.matchAll(INLINE)) {
      parent.append(text.slice(last, match.index));
      let end = match.index + match[0].length;
      if (match[1] !== undefined) {
        const code = document.createElement("code");
        code.textContent = match[1];
        parent.append(code);
      } else if (match[2] !== undefined) {
        const strong = document.createElement("strong");
        strong.textContent = match[2];
        parent.append(strong);
      } else if (match[3] !== undefined) {
        const em = document.createElement("em");
        em.textContent = match[3];
        parent.append(em);
      } else {
        // La puntuación final de una frase («…/b,») no es parte de la dirección.
        const url = match[4].replace(/[.,;:!?]+$/, "");
        end = match.index + url.length;
        const link = document.createElement("a");
        link.className = "link";
        link.href = "#";
        link.textContent = url;
        link.addEventListener("click", (event) => {
          event.preventDefault();
          onLink(url);
        });
        parent.append(link);
      }
      last = end;
    }
    parent.append(text.slice(last));
  }

  const BULLET = /^\s*[-*•]\s+(.*)$/;
  const NUMBERED = /^\s*\d{1,3}[.)]\s+(.*)$/;
  const HEADING = /^#{1,3}\s+(.*)$/;

  function renderRich(container, text, onLink) {
    const lines = text.replace(/\r\n?/g, "\n").split("\n");
    let paragraph = [];
    let list = null; // { tag, element }

    const flushParagraph = () => {
      if (!paragraph.length) return;
      const p = document.createElement("p");
      appendInline(p, paragraph.join("\n"), onLink);
      container.append(p);
      paragraph = [];
    };

    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];

      if (/^\s*```/.test(line)) {
        flushParagraph();
        list = null;
        const code = [];
        i++;
        while (i < lines.length && !/^\s*```/.test(lines[i])) code.push(lines[i++]);
        const pre = document.createElement("pre");
        const inner = document.createElement("code");
        inner.textContent = code.join("\n");
        pre.append(inner);
        container.append(pre);
        continue;
      }

      const bullet = BULLET.exec(line);
      const numbered = bullet ? null : NUMBERED.exec(line);
      if (bullet || numbered) {
        flushParagraph();
        const tag = bullet ? "ul" : "ol";
        if (!list || list.tag !== tag) {
          list = { tag, element: document.createElement(tag) };
          container.append(list.element);
        }
        const item = document.createElement("li");
        appendInline(item, (bullet ?? numbered)[1], onLink);
        list.element.append(item);
        continue;
      }
      list = null;

      const heading = HEADING.exec(line);
      if (heading) {
        flushParagraph();
        const h = document.createElement("div");
        h.className = "heading";
        appendInline(h, heading[1], onLink);
        container.append(h);
        continue;
      }

      if (line.trim() === "") flushParagraph();
      else paragraph.push(line);
    }
    flushParagraph();
  }

  window.renderRich = renderRich;
})();
