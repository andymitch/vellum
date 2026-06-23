<script lang="ts">
  import Self from "./Tree.svelte";
  import type { TreeNode } from "$lib/vault";
  import { ChevronRight, ChevronDown, FileText, Folder } from "@lucide/svelte";

  let {
    nodes,
    activePath,
    expanded,
    onselect,
    onmenu,
    depth = 0,
  }: {
    nodes: TreeNode[];
    activePath: string | null;
    expanded: Record<string, boolean>;
    onselect: (node: TreeNode) => void;
    onmenu: (e: MouseEvent, node: TreeNode) => void;
    depth?: number;
  } = $props();
</script>

{#each nodes as node (node.path)}
  <button
    type="button"
    class="flex w-full items-center gap-1 rounded py-1 pr-2 text-left text-sm transition-colors {activePath ===
    node.path
      ? 'bg-primary/15 text-foreground'
      : 'text-muted-foreground hover:bg-muted hover:text-foreground'}"
    style="padding-left: {depth * 12 + 8}px"
    onclick={() =>
      node.is_dir ? (expanded[node.path] = !expanded[node.path]) : onselect(node)}
    oncontextmenu={(e) => onmenu(e, node)}
  >
    {#if node.is_dir}
      {#if expanded[node.path]}
        <ChevronDown size={14} class="shrink-0 opacity-70" />
      {:else}
        <ChevronRight size={14} class="shrink-0 opacity-70" />
      {/if}
      <Folder size={14} class="shrink-0 opacity-80" />
    {:else}
      <FileText size={14} class="ml-[14px] shrink-0 opacity-70" />
    {/if}
    <span class="truncate">{node.name}</span>
  </button>

  {#if node.is_dir && expanded[node.path]}
    <Self
      nodes={node.children}
      {activePath}
      {expanded}
      {onselect}
      {onmenu}
      depth={depth + 1}
    />
  {/if}
{/each}
