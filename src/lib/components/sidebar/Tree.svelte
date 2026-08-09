<script lang="ts">
  import Self from "./Tree.svelte";
  import type { TreeNode } from "$lib/vault";
  import { drag } from "$lib/dnd";
  import {
    ChevronRight,
    ChevronDown,
    FileText,
    Folder,
    ListChecks,
    NotebookPen,
  } from "@lucide/svelte";

  let {
    nodes,
    activePath,
    expanded,
    onselect,
    onmenu,
    onmove,
    dnd = false,
    depth = 0,
    noteTypes = {},
  }: {
    nodes: TreeNode[];
    activePath: string | null;
    expanded: Record<string, boolean>;
    onselect: (node: TreeNode) => void;
    onmenu: (e: MouseEvent, node: TreeNode) => void;
    onmove?: (from: string, isDir: boolean, toDir: string) => void;
    dnd?: boolean;
    depth?: number;
    // path -> note type, for the per-type icon (#180/#181). Absent means plain
    // Markdown, which is the default and the vast majority.
    noteTypes?: Record<string, string>;
  } = $props();

  // Icon per note type, so a TODO and a journal are distinguishable at a glance
  // in the tree rather than all reading as generic documents.
  const ICONS: Record<string, typeof FileText> = {
    todo: ListChecks,
    // NotebookPen, not NotepadText: at tree size a lined pad reads almost
    // identically to the plain-note page icon (#190). This is the same icon the
    // empty state uses as its watermark, so it's already familiar.
    journal: NotebookPen,
    // `scratchpad` is the pre-#181 name, still readable from beta-era notes.
    scratchpad: NotebookPen,
  };
  const iconFor = (path: string) => ICONS[noteTypes[path]] ?? FileText;

  // Path currently hovered as a drop target (for highlight). Each Tree instance
  // tracks its own; only one node sits under the cursor at a time.
  let dragOver = $state<string | null>(null);
  const dirOf = (p: string) => p.split("/").slice(0, -1).join("/");
  // Files drop into their containing folder; folders drop into themselves.
  const dropDir = (node: TreeNode) => (node.is_dir ? node.path : dirOf(node.path));

  function onDragStart(e: DragEvent, node: TreeNode) {
    drag.item = { path: node.path, is_dir: node.is_dir };
    // WebKit needs setData called or it won't start the drag at all.
    e.dataTransfer?.setData("text/plain", node.path);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
  }
  function onDrop(e: DragEvent, node: TreeNode) {
    e.preventDefault();
    e.stopPropagation();
    dragOver = null;
    const d = drag.item;
    drag.item = null;
    if (d) onmove?.(d.path, d.is_dir, dropDir(node));
  }
</script>

{#each nodes as node (node.path)}
  <button
    type="button"
    draggable={dnd}
    class="flex w-full items-center gap-1.5 rounded py-1.5 pr-2 text-left text-[15px] transition-colors md:gap-1 md:py-1 md:text-sm {dnd
      ? '[&_*]:pointer-events-none'
      : ''} {dragOver === node.path
      ? 'bg-muted text-foreground'
      : activePath === node.path
        ? 'bg-primary/15 text-foreground'
        : 'text-muted-foreground hover:bg-muted hover:text-foreground'}"
    style="padding-left: {depth * 12 + 8}px;{dnd ? '-webkit-user-drag:element;' : ''}"
    onclick={() =>
      node.is_dir ? (expanded[node.path] = !expanded[node.path]) : onselect(node)}
    oncontextmenu={(e) => onmenu(e, node)}
    ondragstart={dnd ? (e) => onDragStart(e, node) : undefined}
    ondragend={dnd
      ? () => {
          dragOver = null;
          drag.item = null;
        }
      : undefined}
    ondragenter={dnd
      ? (e) => {
          e.preventDefault();
          e.stopPropagation();
        }
      : undefined}
    ondragover={dnd
      ? (e) => {
          e.preventDefault();
          e.stopPropagation();
          if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
          dragOver = node.path;
        }
      : undefined}
    ondragleave={dnd ? () => (dragOver = null) : undefined}
    ondrop={dnd ? (e) => onDrop(e, node) : undefined}
  >
    {#if node.is_dir}
      {#if expanded[node.path]}
        <ChevronDown class="h-[18px] w-[18px] shrink-0 opacity-70 md:h-3.5 md:w-3.5" />
      {:else}
        <ChevronRight class="h-[18px] w-[18px] shrink-0 opacity-70 md:h-3.5 md:w-3.5" />
      {/if}
      <Folder class="h-[18px] w-[18px] shrink-0 opacity-80 md:h-3.5 md:w-3.5" />
    {:else}
      {@const Icon = iconFor(node.path)}
      <Icon class="ml-[18px] h-[18px] w-[18px] shrink-0 opacity-70 md:ml-[14px] md:h-3.5 md:w-3.5" />
    {/if}
    <span class="truncate">{node.is_dir ? node.name : node.name.replace(/\.md$/, "")}</span>
  </button>

  {#if node.is_dir && expanded[node.path]}
    <Self
      nodes={node.children}
      {activePath}
      {expanded}
      {onselect}
      {onmenu}
      {onmove}
      {dnd}
      {noteTypes}
      depth={depth + 1}
    />
  {/if}
{/each}
