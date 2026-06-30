<script lang="ts">
  import type { EditorView } from "@codemirror/view";
  import { onMount } from "svelte";
  import {
    Bold,
    Italic,
    Strikethrough,
    Heading,
    List,
    ListChecks,
    Quote,
    Code,
    SquareCode,
    Link,
    IndentIncrease,
    IndentDecrease,
  } from "@lucide/svelte";
  import {
    wrapInline,
    toggleLinePrefix,
    insertLink,
    toggleCodeBlock,
    indent,
    outdent,
  } from "./markdown-actions";

  let { view }: { view: EditorView | undefined } = $props();

  let barEl: HTMLDivElement;

  // Anchor the bar just above the soft keyboard. visualViewport shrinks when the
  // keyboard opens; the occluded height is innerHeight - height - offsetTop.
  let kbInset = $state(0);
  onMount(() => {
    const vv = window.visualViewport;
    if (!vv) return;
    const root = document.documentElement;
    const update = () => {
      kbInset = Math.max(0, window.innerHeight - vv.height - vv.offsetTop);
      // Publish the editor's bottom inset = keyboard occlusion (the editor
      // scroller extends behind the keyboard) + the toolbar's own height. The
      // editor reserves this much space so the caret never scrolls behind the
      // keyboard or the toolbar while typing.
      root.style.setProperty("--editor-kb-inset", `${kbInset + (barEl?.offsetHeight ?? 44)}px`);
    };
    update();
    vv.addEventListener("resize", update);
    vv.addEventListener("scroll", update);
    return () => {
      root.style.setProperty("--editor-kb-inset", "0px");
      vv.removeEventListener("resize", update);
      vv.removeEventListener("scroll", update);
    };
  });

  const actions = [
    { icon: Bold, label: "Bold", run: (v: EditorView) => wrapInline(v, "**") },
    { icon: Italic, label: "Italic", run: (v: EditorView) => wrapInline(v, "*") },
    { icon: Strikethrough, label: "Strikethrough", run: (v: EditorView) => wrapInline(v, "~~") },
    { icon: Heading, label: "Heading", run: (v: EditorView) => toggleLinePrefix(v, "## ") },
    { icon: List, label: "Bullet list", run: (v: EditorView) => toggleLinePrefix(v, "- ") },
    { icon: ListChecks, label: "Checkbox", run: (v: EditorView) => toggleLinePrefix(v, "- [ ] ") },
    { icon: Quote, label: "Quote", run: (v: EditorView) => toggleLinePrefix(v, "> ") },
    { icon: Code, label: "Inline code", run: (v: EditorView) => wrapInline(v, "`") },
    { icon: SquareCode, label: "Code block", run: (v: EditorView) => toggleCodeBlock(v) },
    { icon: Link, label: "Link", run: (v: EditorView) => insertLink(v) },
    { icon: IndentDecrease, label: "Outdent", run: (v: EditorView) => outdent(v) },
    { icon: IndentIncrease, label: "Indent", run: (v: EditorView) => indent(v) },
  ];
</script>

<div
  bind:this={barEl}
  class="fixed inset-x-0 z-40 flex items-center gap-0.5 overflow-x-auto border-t border-border bg-background/95 px-1 py-1 backdrop-blur"
  style="bottom: {kbInset}px;"
  role="toolbar"
  aria-label="Markdown formatting"
>
  {#each actions as a (a.label)}
    {@const Icon = a.icon}
    <button
      type="button"
      class="shrink-0 rounded-md p-2 text-muted-foreground active:bg-muted active:text-foreground"
      aria-label={a.label}
      title={a.label}
      onpointerdown={(e) => e.preventDefault()}
      onclick={() => view && a.run(view)}
    >
      <Icon size={18} />
    </button>
  {/each}
</div>
