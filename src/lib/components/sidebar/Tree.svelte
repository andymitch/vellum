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
    class="flex w-full items-center gap-1.5 rounded py-1.5 pr-2 text-left text-[15px] transition-colors md:gap-1 md:py-1 md:text-sm {activePath ===
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
        <ChevronDown class="h-[18px] w-[18px] shrink-0 opacity-70 md:h-3.5 md:w-3.5" />
      {:else}
        <ChevronRight class="h-[18px] w-[18px] shrink-0 opacity-70 md:h-3.5 md:w-3.5" />
      {/if}
      <Folder class="h-[18px] w-[18px] shrink-0 opacity-80 md:h-3.5 md:w-3.5" />
    {:else}
      <FileText class="ml-[18px] h-[18px] w-[18px] shrink-0 opacity-70 md:ml-[14px] md:h-3.5 md:w-3.5" />
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
      depth={depth + 1}
    />
  {/if}
{/each}
