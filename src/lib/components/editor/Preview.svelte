<script lang="ts">
  import { Marked } from "marked";
  import { markedHighlight } from "marked-highlight";
  import hljs from "highlight.js/lib/common";

  let { value = $bindable("") }: { value?: string } = $props();

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
</script>

<div class="md-preview" onchange={onToggle}>
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
  /* Bigger, theme-colored, interactive task checkboxes. */
  .md-preview :global(li > input[type="checkbox"]) {
    width: 1.15em;
    height: 1.15em;
    margin-right: 0.45em;
    accent-color: var(--editor-accent);
    cursor: pointer;
    vertical-align: -0.18em;
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
