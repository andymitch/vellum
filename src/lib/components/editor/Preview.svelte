<script lang="ts">
  import { openExternal } from "$lib/host";
  import { theme } from "$lib/theme.svelte";
  import type { Mermaid } from "mermaid";

  import { slugify } from "$lib/slug";
  import { parseNote } from "$lib/note-type";
  import { noteCard } from "$lib/link-card";
  import { fetchLinkPreview } from "$lib/vault";
  import { editorSettings } from "$lib/editor-settings.svelte";
  import { renderMarkdown, resolveWikiLink } from "$lib/render-markdown";

  let {
    value = $bindable(""),
    notePaths = [],
    // Open an internal link's resolved note path, optionally scrolling to a
    // heading anchor (#45). A same-note `[[#heading]]` link is handled here.
    oninternallink,
    // Clicking a #tag chip opens the search palette filtered to that tag (#15).
    ontag,
    // Read a note's Markdown, for the excerpt on an internal link's preview card
    // (#62). Supplied by the parent because the vault id lives there.
    loadNote,
  }: {
    value?: string;
    notePaths?: string[];
    oninternallink?: (path: string, fragment?: string) => void;
    ontag?: (tag: string) => void;
    loadNote?: (path: string) => Promise<string>;
  } = $props();

  // The frontmatter block carries the note's type (#104) and is chrome, not
  // content — strip it before rendering. Only a LEADING block is stripped, so a
  // scratchpad's `---` separators still render as thematic breaks.
  const html = $derived(renderMarkdown(parseNote(value).body));

  // Flip the source marker for the checkbox that changed. Task checkboxes render
  // in source order, so the Nth checkbox in the DOM maps to the Nth `[ ]`/`[x]`
  // marker in the markdown. Rewriting `value` re-renders to the new state.
  function onToggle(e: Event) {
    const t = e.target;
    if (!(t instanceof HTMLInputElement) || t.type !== "checkbox") return;
    const container = e.currentTarget as HTMLElement;
    const boxes = [...container.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')];
    const index = boxes.indexOf(t);
    if (index < 0) return;
    let i = 0;
    value = value.replace(
      /^([ \t]*(?:[-*+]|\d+[.)])[ \t]+\[)([ xX])(\])/gm,
      (m, pre, mark, post) => (i++ === index ? pre + (mark === " " ? "x" : " ") + post : m),
    );
  }

  // ---- Internal links (#16) ----
  let container: HTMLDivElement;

  // After each render (or when the note list changes): give headings stable slug
  // ids (deduped), then resolve every wiki link — a note target, a same-note
  // #heading, or both — attaching the matched path or marking it broken.
  $effect(() => {
    html;
    notePaths;
    const root = container;
    if (!root) return;
    const seen = new Map<string, number>();
    for (const h of root.querySelectorAll<HTMLElement>("h1,h2,h3,h4,h5,h6")) {
      const base = slugify(h.textContent ?? "");
      if (!base) continue;
      const n = seen.get(base) ?? 0;
      seen.set(base, n + 1);
      h.id = n ? `${base}-${n}` : base;
    }
    for (const a of root.querySelectorAll<HTMLAnchorElement>("a.wikilink")) {
      const target = a.dataset.target ?? "";
      const fragment = a.dataset.fragment ?? "";
      let ok = false;
      if (target) {
        const path = resolveWikiLink(target, notePaths);
        if (path) {
          a.dataset.path = path;
          ok = true;
        } else delete a.dataset.path;
      } else if (fragment) {
        // Same-note anchor — valid when the heading exists in this render.
        delete a.dataset.path;
        ok = !!root.querySelector(`#${CSS.escape(slugify(fragment))}`);
      }
      a.classList.toggle("broken", !ok);
      a.title = ok
        ? fragment
          ? `${a.dataset.path ?? ""}#${fragment}`
          : (a.dataset.path ?? "")
        : target
          ? `No note matches "${target}"`
          : `No section "${fragment}"`;
    }
  });

  // The webview would otherwise navigate the whole app to an external link or
  // open it in an in-app view. Internal wiki links open another note; external
  // (absolute/mailto) links go to the OS default browser via the opener plugin.
  function onClick(e: MouseEvent) {
    const a = (e.target as HTMLElement | null)?.closest("a");
    if (!a) return;
    if (a.classList.contains("tagchip")) {
      e.preventDefault();
      const tag = a.dataset.tag;
      if (tag) ontag?.(tag);
      return;
    }
    if (a.classList.contains("wikilink")) {
      e.preventDefault();
      const path = a.dataset.path;
      const fragment = a.dataset.fragment || undefined;
      if (path) oninternallink?.(path, fragment);
      else if (fragment)
        container
          ?.querySelector(`#${CSS.escape(slugify(fragment))}`)
          ?.scrollIntoView({ behavior: "smooth", block: "start" });
      return;
    }
    const href = a.getAttribute("href");
    if (!href || !/^(https?:|mailto:)/i.test(href)) return;
    e.preventDefault();
    openExternal(href);
  }

  // ---- Mermaid diagrams -------------------------------------------------
  // ```mermaid fences render as a normal code block first (hljs as plaintext),
  // then this effect swaps each one for an SVG. mermaid is ~1MB, so it's
  // dynamically imported the first time a diagram actually appears.
  // (`container` is declared above for internal-link resolution.)
  let mermaidLib: Mermaid | undefined;
  let mermaidThemeDark: boolean | undefined; // theme mermaid was last initialized for
  let renderSeq = 0;

  // Empties `placeholder` and hides it rather than removing it from the tree,
  // then inserts `replacement` in its place. `placeholder` (a `pre` or `p`
  // from the parsed markdown) is a top-level child of the {@html} block, and
  // Svelte tracks that block's first/last child by reference to know what to
  // clear when the note's content next changes. Detaching it via replaceWith
  // stales that reference, so a later note switch can't find where its
  // content ends — the diagram or link card then leaks into whatever note is
  // opened next (#228).
  function swapInPlace(placeholder: HTMLElement, replacement: HTMLElement) {
    placeholder.replaceChildren();
    placeholder.style.display = "none";
    placeholder.before(replacement);
  }

  async function renderDiagram(mermaid: Mermaid, src: string, id: string) {
    try {
      const { svg } = await mermaid.render(id, src);
      return svg;
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      return `<pre class="mermaid-error">${msg.replace(/[<>&]/g, (c) => ({ "<": "&lt;", ">": "&gt;", "&": "&amp;" })[c]!)}</pre>`;
    }
  }

  async function renderMermaid() {
    // Claim a sequence number up front (before any early return or await) so an
    // in-flight render always sees a newer renderSeq and aborts — even when the
    // new pass has no diagrams to render.
    const seq = ++renderSeq;
    const root = container;
    if (!root) return;
    // Unrendered fences from a fresh {@html}, plus already-rendered diagrams
    // that need re-theming when light/dark flips (their source is on data-src).
    const fresh = [...root.querySelectorAll<HTMLElement>("pre > code.language-mermaid")];
    const themed = [...root.querySelectorAll<HTMLElement>(".mermaid-diagram[data-src]")];
    if (!fresh.length && !themed.length) return;

    if (!mermaidLib) {
      mermaidLib = (await import("mermaid")).default;
      if (seq !== renderSeq) return;
    }
    const mermaid = mermaidLib;
    // initialize() resets mermaid's global state, so only call it when the theme
    // actually changed — calling it on every render can clobber a concurrent
    // render. securityLevel "strict" sanitizes the SVG; we still own the input.
    const dark = theme.dark;
    if (mermaidThemeDark !== dark) {
      mermaid.initialize({ startOnLoad: false, securityLevel: "strict", theme: dark ? "dark" : "default" });
      mermaidThemeDark = dark;
    }

    let i = 0;
    // Render to a string first, then re-check seq before touching live DOM, so a
    // superseded render never flashes stale SVG into an on-screen node.
    for (const codeEl of fresh) {
      const pre = codeEl.closest("pre");
      if (!pre) continue;
      const src = codeEl.textContent ?? "";
      const svg = await renderDiagram(mermaid, src, `mmd-${seq}-${i++}`);
      if (seq !== renderSeq) return;
      const wrap = document.createElement("div");
      wrap.className = "mermaid-diagram";
      wrap.dataset.src = src;
      wrap.innerHTML = svg;
      swapInPlace(pre, wrap);
    }
    for (const wrap of themed) {
      const svg = await renderDiagram(mermaid, wrap.dataset.src ?? "", `mmd-${seq}-${i++}`);
      if (seq !== renderSeq) return;
      wrap.innerHTML = svg;
    }
  }

  // Re-run when the rendered HTML changes (new/changed diagrams) or the theme
  // flips (existing diagrams need recoloring).
  $effect(() => {
    html;
    theme.dark;
    renderMermaid();
  });

  // ---- Link preview cards (#62) -----------------------------------------
  // A link that sits ALONE on its line becomes a card; a link inside a sentence
  // stays inline text. That rule is the whole reason this runs on the rendered
  // DOM rather than in a marked extension: "alone on its line" is exactly "the
  // paragraph's only content", which marked can only tell us after inline
  // parsing. It also keeps prose flow untouched, which was the failure mode of
  // replacing links wherever they appear.
  let cardSeq = 0;

  /// The single anchor a paragraph consists of, or null if it has any other
  /// content. Whitespace-only text nodes don't count as content.
  function loneLink(p: HTMLElement): HTMLAnchorElement | null {
    let found: HTMLAnchorElement | null = null;
    for (const node of p.childNodes) {
      if (node.nodeType === Node.TEXT_NODE) {
        if ((node.textContent ?? "").trim()) return null;
        continue;
      }
      if (node instanceof HTMLAnchorElement && !found) {
        found = node;
        continue;
      }
      return null; // a second anchor, or any other element
    }
    return found;
  }

  function cardShell(href: string | null, cls: string): HTMLAnchorElement {
    const card = document.createElement("a");
    card.className = `link-card ${cls}`;
    card.setAttribute("href", href ?? "#");
    return card;
  }

  function fillCard(
    card: HTMLElement,
    parts: { title: string; body?: string | null; meta?: string | null; image?: string | null },
  ) {
    // Built with the DOM API rather than innerHTML: titles and descriptions are
    // third-party strings from a fetched page, and this is the one place in the
    // preview where the content isn't the user's own.
    if (parts.image) {
      const img = document.createElement("img");
      img.className = "link-card-img";
      img.src = parts.image;
      img.alt = "";
      img.loading = "lazy";
      // A broken or blocked image must not leave a torn card.
      img.onerror = () => img.remove();
      card.appendChild(img);
    }
    const text = document.createElement("span");
    text.className = "link-card-text";
    const t = document.createElement("span");
    t.className = "link-card-title";
    t.textContent = parts.title;
    text.appendChild(t);
    if (parts.body) {
      const d = document.createElement("span");
      d.className = "link-card-desc";
      d.textContent = parts.body;
      text.appendChild(d);
    }
    if (parts.meta) {
      const m = document.createElement("span");
      m.className = "link-card-meta";
      m.textContent = parts.meta;
      text.appendChild(m);
    }
    card.appendChild(text);
  }

  async function renderLinkCards() {
    const seq = ++cardSeq;
    const root = container;
    if (!root || !editorSettings.linkPreviews) return;

    for (const p of [...root.querySelectorAll<HTMLElement>("p")]) {
      const a = loneLink(p);
      if (!a) continue;

      // Internal `[[note]]` — fully local, so the card is built immediately.
      if (a.classList.contains("wikilink")) {
        const path = a.dataset.path;
        // A broken link keeps its inline "broken" styling; a card would imply
        // the note exists.
        if (!path || !loadNote) continue;
        const md = await loadNote(path).catch(() => null);
        if (seq !== cardSeq) return;
        if (md === null) continue;
        const { title, excerpt } = noteCard(md, path);
        const card = cardShell("#", "internal");
        card.classList.add("wikilink");
        card.dataset.path = path;
        card.dataset.target = a.dataset.target ?? "";
        card.dataset.fragment = a.dataset.fragment ?? "";
        fillCard(card, { title, body: excerpt, meta: path.replace(/\.md$/i, "") });
        if (!p.isConnected) continue;
        swapInPlace(p, card);
        continue;
      }

      // External http(s) — needs the network, so the plain link stays put until
      // (and unless) metadata actually comes back.
      const href = a.getAttribute("href") ?? "";
      if (!/^https?:\/\//i.test(href)) continue;
      const meta = await fetchLinkPreview(href).catch(() => null);
      if (seq !== cardSeq) return;
      if (!meta) continue; // offline, failed, or nothing worth showing
      // `p` may have been detached by a newer render; swapInPlace on an orphan
      // would hide a node nobody sees, and the seq check above already covers
      // the common case, so this is just belt-and-braces.
      if (!p.isConnected) continue;
      const card = cardShell(href, "external");
      fillCard(card, {
        title: meta.title || href,
        body: meta.description,
        meta: meta.site_name,
        image: meta.image,
      });
      swapInPlace(p, card);
    }
  }

  $effect(() => {
    html;
    notePaths;
    editorSettings.linkPreviews;
    renderLinkCards();
  });
</script>

<!-- onclick delegates link handling to the OS browser; the real <a>s inside are
     keyboard-accessible, so the static-element a11y rules don't apply here. -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="md-preview prose-content" bind:this={container} onchange={onToggle} onclick={onClick}>
  <!-- Keyed on the link-preview setting so turning it OFF restores the plain
       links. Cards are DOM replacements applied after render, and {@html} only
       re-runs when the markdown itself changes — without this key, existing
       cards would linger until the next edit (#62). -->
  {#key editorSettings.linkPreviews}
    <!-- eslint-disable-next-line svelte/no-at-html-tags -->
    {@html html}
  {/key}
</div>

<style>
  /* Layout for a full note preview. The shared typography (headings, lists,
     code, tables, tags, link cards, ...) lives in .prose-content (app.css),
     so JournalView's per-cell read view renders identically without
     duplicating any of it. */
  .md-preview {
    max-width: 96rem;
    margin: 0 auto;
    padding: 1.5rem;
  }
</style>
