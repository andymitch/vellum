<script lang="ts">
  import { onMount } from "svelte";
  import {
    listVaults,
    createVault,
    joinVault,
    shareVault,
    listTree,
    createNote,
    createFolder,
    renamePath,
    deletePath,
    watchVault,
    onVaultChanged,
    type VaultInfo,
    type TreeNode,
  } from "$lib/vault";
  import Tree from "./Tree.svelte";
  import {
    FilePlus,
    FolderPlus,
    Share2,
    Download,
    Plus,
  } from "@lucide/svelte";

  let {
    activePath = null,
    onopen,
    onvaultchange,
  }: {
    activePath?: string | null;
    onopen: (vault: string, path: string) => void;
    onvaultchange: (vault: string | null) => void;
  } = $props();

  let vaults = $state<VaultInfo[]>([]);
  let activeVault = $state<string | null>(null);
  let tree = $state<TreeNode[]>([]);
  let expanded = $state<Record<string, boolean>>({});
  let menu = $state<{ x: number; y: number; node: TreeNode } | null>(null);

  const dirOf = (path: string) => path.split("/").slice(0, -1).join("/");
  const join = (dir: string, name: string) => (dir ? `${dir}/${name}` : name);

  async function refreshVaults() {
    vaults = await listVaults();
  }

  async function refreshTree() {
    tree = activeVault ? await listTree(activeVault) : [];
  }

  async function setActive(id: string | null) {
    activeVault = id;
    expanded = {};
    onvaultchange(id);
    if (id) {
      await watchVault(id);
      await refreshTree();
    } else {
      tree = [];
    }
  }

  async function newVault() {
    const name = window.prompt("New vault name")?.trim();
    if (!name) return;
    const v = await createVault(name);
    await refreshVaults();
    await setActive(v.id);
  }

  async function joinPrompt() {
    const ticket = window.prompt("Paste vault ticket")?.trim();
    if (!ticket) return;
    const v = await joinVault(ticket);
    await refreshVaults();
    await setActive(v.id);
  }

  async function share() {
    if (!activeVault) return;
    const ticket = await shareVault(activeVault);
    // window.prompt ignores its default value in the Tauri webview, so the
    // ticket must be surfaced another way: copy to clipboard + show it.
    let copied = false;
    try {
      await navigator.clipboard.writeText(ticket);
      copied = true;
    } catch {
      copied = false;
    }
    window.alert(
      (copied ? "Vault ticket (copied to clipboard):\n\n" : "Vault ticket:\n\n") +
        ticket,
    );
  }

  async function newRootNote() {
    if (!activeVault) return;
    const name = window.prompt("New note name (e.g. note.md)")?.trim();
    if (!name) return;
    await createNote(activeVault, name);
    await refreshTree();
    onopen(activeVault, name);
  }

  async function newRootFolder() {
    if (!activeVault) return;
    const name = window.prompt("New folder name")?.trim();
    if (!name) return;
    await createFolder(activeVault, name);
    await refreshTree();
  }

  // --- context menu actions ---
  function openMenu(e: MouseEvent, node: TreeNode) {
    e.preventDefault();
    menu = { x: e.clientX, y: e.clientY, node };
  }
  const closeMenu = () => (menu = null);

  async function act(kind: string, node: TreeNode) {
    closeMenu();
    if (!activeVault) return;
    if (kind === "new-note") {
      const name = window.prompt("New note name")?.trim();
      if (!name) return;
      await createNote(activeVault, join(node.path, name));
      expanded[node.path] = true;
    } else if (kind === "new-folder") {
      const name = window.prompt("New folder name")?.trim();
      if (!name) return;
      await createFolder(activeVault, join(node.path, name));
      expanded[node.path] = true;
    } else if (kind === "rename") {
      const name = window.prompt("Rename to", node.name)?.trim();
      if (!name || name === node.name) return;
      const to = join(dirOf(node.path), name);
      await renamePath(activeVault, node.path, to, node.is_dir);
      if (activePath === node.path) onopen(activeVault, to);
    } else if (kind === "delete") {
      if (!window.confirm(`Delete "${node.name}"?`)) return;
      await deletePath(activeVault, node.path, node.is_dir);
      if (activePath && (activePath === node.path || activePath.startsWith(node.path + "/")))
        onvaultchange(activeVault);
    }
    await refreshTree();
  }

  // Retry backend calls during startup until the node is ready.
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

  onMount(() => {
    let unlisten: (() => void) | undefined;
    (async () => {
      // The backend may still be starting (dev: window opens before the Rust
      // binary is up). Retry until it answers.
      for (let i = 0; i < 40; i++) {
        try {
          await refreshVaults();
          break;
        } catch {
          await sleep(250);
        }
      }
      if (vaults.length) await setActive(vaults[0].id);
      unlisten = await onVaultChanged((id) => {
        if (id === activeVault) refreshTree();
      });
    })();
    return () => unlisten?.();
  });
</script>

<svelte:window onclick={closeMenu} />

<div class="flex h-full flex-col">
  <!-- Vault switcher -->
  <div class="flex items-center gap-1 border-b border-border p-2">
    <select
      class="min-w-0 flex-1 truncate rounded border border-border bg-background px-2 py-1 text-sm"
      value={activeVault ?? ""}
      onchange={(e) => setActive((e.currentTarget as HTMLSelectElement).value || null)}
    >
      {#if !vaults.length}
        <option value="">No vaults</option>
      {/if}
      {#each vaults as v (v.id)}
        <option value={v.id}>{v.name}</option>
      {/each}
    </select>
    <button
      type="button"
      class="rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground"
      title="New vault"
      onclick={newVault}
    >
      <Plus size={16} />
    </button>
    <button
      type="button"
      class="rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground"
      title="Join vault from ticket"
      onclick={joinPrompt}
    >
      <Download size={16} />
    </button>
    <button
      type="button"
      class="rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground disabled:opacity-40"
      title="Share vault"
      disabled={!activeVault}
      onclick={share}
    >
      <Share2 size={16} />
    </button>
  </div>

  <!-- Root file/folder actions -->
  <div class="flex items-center justify-between px-2 py-1.5">
    <span class="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
      Files
    </span>
    <div class="flex gap-0.5">
      <button
        type="button"
        class="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground disabled:opacity-40"
        title="New note"
        disabled={!activeVault}
        onclick={newRootNote}
      >
        <FilePlus size={15} />
      </button>
      <button
        type="button"
        class="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground disabled:opacity-40"
        title="New folder"
        disabled={!activeVault}
        onclick={newRootFolder}
      >
        <FolderPlus size={15} />
      </button>
    </div>
  </div>

  <!-- Tree -->
  <div class="min-h-0 flex-1 overflow-auto px-1 pb-2">
    {#if activeVault && tree.length}
      <Tree
        nodes={tree}
        {activePath}
        {expanded}
        onselect={(node) => onopen(activeVault!, node.path)}
        onmenu={openMenu}
      />
    {:else if activeVault}
      <p class="px-2 py-4 text-sm text-muted-foreground">Empty vault.</p>
    {:else}
      <p class="px-2 py-4 text-sm text-muted-foreground">
        Create or join a vault to begin.
      </p>
    {/if}
  </div>
</div>

<!-- Context menu -->
{#if menu}
  {@const node = menu.node}
  <div
    class="fixed z-50 min-w-36 rounded-md border border-border bg-popover py-1 text-sm shadow-lg"
    style="left: {menu.x}px; top: {menu.y}px"
    role="menu"
    tabindex="-1"
  >
    {#if node.is_dir}
      <button class="block w-full px-3 py-1 text-left hover:bg-muted" onclick={() => act("new-note", node)}>New note</button>
      <button class="block w-full px-3 py-1 text-left hover:bg-muted" onclick={() => act("new-folder", node)}>New folder</button>
    {/if}
    <button class="block w-full px-3 py-1 text-left hover:bg-muted" onclick={() => act("rename", node)}>Rename</button>
    <button class="block w-full px-3 py-1 text-left text-destructive hover:bg-muted" onclick={() => act("delete", node)}>Delete</button>
  </div>
{/if}
