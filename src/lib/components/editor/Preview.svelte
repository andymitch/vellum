<script lang="ts">
  import { Marked, type TokenizerAndRendererExtension } from "marked";
  import { markedHighlight } from "marked-highlight";
  import hljs from "highlight.js/lib/common";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { theme } from "$lib/theme.svelte";
  import type { Mermaid } from "mermaid";

  let {
    value = $bindable(""),
    // All note paths in the current vault, used to resolve [[wiki links]].
    notePaths = [],
    // Open an internal link's resolved note path.
    oninternallink,
  }: {
    value?: string;
    notePaths?: string[];
    oninternallink?: (path: string) => void;
  } = $props();

  const escHtml = (s: string) =>
    s.replace(/[<>&]/g, (c) => ({ "<": "&lt;", ">": "&gt;", "&": "&amp;" })[c]!);
  const escAttr = (s: string) => escHtml(s).replace(/"/g, "&quot;");

  // `[[target]]` or `[[target|label]]` → an internal link. Resolution against
  // the vault's notes happens after render (see the effect below), since the
  // note list is reactive and lives outside this marked instance.
  const wikiLink: TokenizerAndRendererExtension = {
    name: "wikilink",
    level: "inline",
    start(src) {
      return src.indexOf("[[");
    },
    tokenizer(src) {
      const m = /^\[\[([^\]\n]+?)\]\]/.exec(src);
      if (!m) return;
      const [target, label] = m[1].split("|");
      return {
        type: "wikilink",
        raw: m[0],
        target: target.trim(),
        label: (label ?? target).trim(),
      };
    },
    renderer(token) {
      return `<a href="#" class="wikilink" data-target="${escAttr(token.target)}">${escHtml(token.label)}</a>`;
    },
  };

  // Local Marked instance with highlight.js. We emit hljs token classes and
  // style them via our --code-* vars (same palette as the editor), so no
  // highlight.js theme stylesheet is imported.
  const marked = new Marked(
    { gfm: true, breaks: false },
    markedHighlight({
      langPrefix: "hljs language-",
      highlight(code, lang) {
        const language = hljs.getLanguage(lang) ? lang : "plaintext";
        return hljs.highlight(code, { language }).value;
      },
    }),
  );
  marked.use({ extensions: [wikiLink] });

  // Content is the user's own local notes, rendered in a desktop app — no
  // untrusted input — so we render marked output directly. GFM task-list
  // checkboxes are rendered `disabled` by marked; strip that so they're
  // interactive (toggling rewrites the source — see onToggle).
  const html = $derived(
    (marked.parse(value) as string).replace(/(<input\b[^>]*?)\s+disabled(?:="")?/g, "$1"),
  );

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

  const stripExt = (s: string) => s.replace(/\.[a-z0-9]+$/i, "");

  // Resolve a wiki-link target to an actual note path: try the path as-is and
  // with a .md extension (exact, then case-insensitive), then fall back to a
  // basename match so `[[todo]]` finds `work/todo.md`.
  function resolveLink(target: string): string | null {
    const t = target.replace(/\\/g, "/").replace(/^\/+/, "");
    if (!t) return null;
    const withMd = /\.[a-z0-9]+$/i.test(t) ? t : `${t}.md`;
    if (notePaths.includes(t)) return t;
    if (notePaths.includes(withMd)) return withMd;
    const lc = withMd.toLowerCase();
    const ci = notePaths.find((p) => p.toLowerCase() === lc);
    if (ci) return ci;
    const base = stripExt(t.split("/").pop() ?? "").toLowerCase();
    return notePaths.find((p) => stripExt(p.split("/").pop() ?? "").toLowerCase() === base) ?? null;
  }

  // After each render (or when the note list changes), resolve every wiki link:
  // attach the matched path, or mark it broken when nothing matches.
  $effect(() => {
    html;
    notePaths;
    const root = container;
    if (!root) return;
    for (const a of root.querySelectorAll<HTMLAnchorElement>("a.wikilink")) {
      const path = resolveLink(a.dataset.target ?? "");
      if (path) {
        a.dataset.path = path;
        a.classList.remove("broken");
        a.title = path;
      } else {
        delete a.dataset.path;
        a.classList.add("broken");
        a.title = `No note matches "${a.dataset.target ?? ""}"`;
      }
    }
  });

  // The webview would otherwise navigate the whole app to an external link or
  // open it in an in-app view. Internal wiki links open another note; external
  // (absolute/mailto) links go to the OS default browser via the opener plugin.
  function onClick(e: MouseEvent) {
    const a = (e.target as HTMLElement | null)?.closest("a");
    if (!a) return;
    if (a.classList.contains("wikilink")) {
      e.preventDefault();
      const path = a.dataset.path;
      if (path) oninternallink?.(path);
      return;
    }
    const href = a.getAttribute("href");
    if (!href || !/^(https?:|mailto:)/i.test(href)) return;
    e.preventDefault();
    openUrl(href);
  }

  // ---- Mermaid diagrams -------------------------------------------------
  // ```mermaid fences render as a normal code block first (hljs as plaintext),
  // then this effect swaps each one for an SVG. mermaid is ~1MB, so it's
  // dynamically imported the first time a diagram actually appears.
  // (`container` is declared above for internal-link resolution.)
  let mermaidLib: Mermaid | undefined;
  let mermaidThemeDark: boolean | undefined; // theme mermaid was last initialized for
  let renderSeq = 0;

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
      const svg = await renderDiagram(mermaid, codeEl.textContent ?? "", `mmd-${seq}-${i++}`);
      if (seq !== renderSeq) return;
      const wrap = document.createElement("div");
      wrap.className = "mermaid-diagram";
      wrap.dataset.src = codeEl.textContent ?? "";
      wrap.innerHTML = svg;
      pre.replaceWith(wrap);
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
</script>

<!-- onclick delegates link handling to the OS browser; the real <a>s inside are
     keyboard-accessible, so the static-element a11y rules don't apply here. -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="md-preview" bind:this={container} onchange={onToggle} onclick={onClick}>
  <!-- eslint-disable-next-line svelte/no-at-html-tags -->
  {@html html}
</div>

<style>
  .md-preview {
    max-width: 48rem;
    margin: 0 auto;
    padding: 1.5rem;
    line-height: 1.7;
    color: var(--editor-fg);
    font-family: var(--font-sans);
  }

  /* No dead space above the first block (headings carry a top margin). */
  .md-preview :global(> :first-child) {
    margin-top: 0;
  }

  .md-preview :global(h1),
  .md-preview :global(h2),
  .md-preview :global(h3),
  .md-preview :global(h4),
  .md-preview :global(h5),
  .md-preview :global(h6) {
    font-weight: 700;
    line-height: 1.25;
    margin: 1.4em 0 0.5em;
  }
  /* Things colors headings per level; H1/H6 stay neutral */
  .md-preview :global(h1) {
    font-size: 1.9em;
    color: var(--editor-fg);
  }
  .md-preview :global(h2) {
    font-size: 1.5em;
    color: var(--md-h2);
  }
  .md-preview :global(h3) {
    font-size: 1.25em;
    color: var(--md-h3);
  }
  .md-preview :global(h4) {
    color: var(--md-h4);
  }
  .md-preview :global(h5) {
    color: var(--md-h5);
  }
  .md-preview :global(h6) {
    color: var(--editor-muted);
  }

  .md-preview :global(p) {
    margin: 0.75em 0;
  }

  .md-preview :global(strong),
  .md-preview :global(b) {
    color: var(--md-strong);
  }
  .md-preview :global(em),
  .md-preview :global(i) {
    color: var(--md-em);
  }

  .md-preview :global(a) {
    color: var(--editor-accent);
    text-decoration: underline;
  }

  /* Internal [[wiki links]]: a subtle pill, dashed + muted when unresolved. */
  .md-preview :global(a.wikilink) {
    text-decoration: none;
    border-bottom: 1px solid color-mix(in srgb, var(--editor-accent) 45%, transparent);
    cursor: pointer;
  }
  .md-preview :global(a.wikilink.broken) {
    color: var(--editor-muted);
    border-bottom-style: dashed;
    cursor: help;
  }

  /* Tailwind's preflight resets list-style to none, so restore markers. */
  .md-preview :global(ul),
  .md-preview :global(ol) {
    padding-left: 1.5em;
    margin: 0.75em 0;
  }
  .md-preview :global(ul) {
    list-style: disc outside;
  }
  .md-preview :global(ol) {
    list-style: decimal outside;
  }
  .md-preview :global(ul ul) {
    list-style-type: circle;
  }
  .md-preview :global(ul ul ul) {
    list-style-type: square;
  }
  .md-preview :global(li) {
    margin: 0.2em 0;
  }
  /* GFM task lists keep their checkboxes, no bullet. */
  .md-preview :global(li:has(> input[type="checkbox"])) {
    list-style: none;
    margin-left: -1.2em;
  }
  /* A sublist nested under a checkbox line must cancel that -1.2em pull-back so
     its bullets stay indented under the task text, not flush with it. */
  .md-preview :global(li:has(> input[type="checkbox"]) > ul),
  .md-preview :global(li:has(> input[type="checkbox"]) > ol) {
    margin-left: 1.2em;
  }
  /* Theme-aware task checkboxes: a subtle "off-background" box that respects the
     active palette, with the checked state drawn in the theme accent. */
  .md-preview :global(li > input[type="checkbox"]) {
    appearance: none;
    -webkit-appearance: none;
    width: 1.15em;
    height: 1.15em;
    margin-right: 0.45em;
    vertical-align: -0.22em;
    border: 1.5px solid var(--editor-border);
    border-radius: 0.3em;
    background: var(--secondary);
    cursor: pointer;
    position: relative;
  }
  /* Checked: solid fill in the primary color (matching the source/preview
     toggle), with the checkmark drawn in the contrasting foreground. */
  .md-preview :global(li > input[type="checkbox"]:checked) {
    background: var(--primary);
    border-color: var(--primary);
  }
  .md-preview :global(li > input[type="checkbox"]:checked)::after {
    content: "";
    position: absolute;
    left: 0.38em;
    top: 0.11em;
    width: 0.3em;
    height: 0.62em;
    border: solid var(--primary-foreground);
    border-width: 0 0.2em 0.2em 0;
    transform: rotate(45deg);
  }
  /* Fade a completed task line (the whole row, checkbox included). */
  .md-preview :global(li:has(> input[type="checkbox"]:checked)) {
    opacity: 0.55;
  }

  .md-preview :global(blockquote) {
    margin: 0.75em 0;
    padding-left: 1em;
    border-left: 3px solid var(--md-quote);
    color: var(--editor-muted);
  }

  .md-preview :global(code) {
    font-family: var(--font-mono);
    font-size: 0.9em;
    background: var(--editor-code-bg);
    border-radius: 4px;
    padding: 0.1em 0.35em;
  }

  .md-preview :global(pre) {
    background: var(--editor-code-bg);
    border-radius: 8px;
    padding: 1em;
    overflow-x: auto;
  }
  .md-preview :global(pre code) {
    background: none;
    padding: 0;
  }

  .md-preview :global(hr) {
    border: none;
    border-top: 1px solid var(--editor-border);
    margin: 1.5em 0;
  }

  /* Rendered Mermaid diagrams: centered, scaled to fit, scroll if too wide. */
  .md-preview :global(.mermaid-diagram) {
    margin: 1em 0;
    text-align: center;
    overflow-x: auto;
  }
  .md-preview :global(.mermaid-diagram svg) {
    max-width: 100%;
    height: auto;
  }
  /* A diagram that failed to parse falls back to its error text. */
  .md-preview :global(.mermaid-error) {
    color: var(--destructive, #e5484d);
    white-space: pre-wrap;
  }

  /* highlight.js tokens mapped to the editor's --code-* palette */
  .md-preview :global(.hljs) {
    color: var(--editor-fg);
  }
  .md-preview :global(.hljs-comment),
  .md-preview :global(.hljs-quote) {
    color: var(--code-comment);
    font-style: italic;
  }
  .md-preview :global(.hljs-keyword),
  .md-preview :global(.hljs-selector-tag),
  .md-preview :global(.hljs-literal),
  .md-preview :global(.hljs-section),
  .md-preview :global(.hljs-doctag),
  .md-preview :global(.hljs-name) {
    color: var(--code-keyword);
  }
  .md-preview :global(.hljs-string),
  .md-preview :global(.hljs-regexp),
  .md-preview :global(.hljs-meta .hljs-string),
  .md-preview :global(.hljs-symbol),
  .md-preview :global(.hljs-bullet) {
    color: var(--code-string);
  }
  .md-preview :global(.hljs-number),
  .md-preview :global(.hljs-link) {
    color: var(--code-number);
  }
  .md-preview :global(.hljs-title),
  .md-preview :global(.hljs-title.function_),
  .md-preview :global(.hljs-function .hljs-title) {
    color: var(--code-function);
  }
  .md-preview :global(.hljs-type),
  .md-preview :global(.hljs-built_in),
  .md-preview :global(.hljs-title.class_),
  .md-preview :global(.hljs-class .hljs-title) {
    color: var(--code-type);
  }
  .md-preview :global(.hljs-variable),
  .md-preview :global(.hljs-template-variable),
  .md-preview :global(.hljs-attr),
  .md-preview :global(.hljs-attribute),
  .md-preview :global(.hljs-property),
  .md-preview :global(.hljs-params) {
    color: var(--code-variable);
  }
</style>
