<script lang="ts">
  import { onMount } from "svelte";
  import Editor from "$lib/components/editor/Editor.svelte";
  import Preview from "$lib/components/editor/Preview.svelte";
  import Sidebar from "$lib/components/sidebar/Sidebar.svelte";
  import { readNote, writeNote, onVaultChanged } from "$lib/vault";
  import { Code, Eye, type Icon as IconType } from "@lucide/svelte";

  type Mode = "source" | "preview";
  const modes: { id: Mode; label: string; icon: typeof IconType }[] = [
    { id: "source", label: "Source", icon: Code },
    { id: "preview", label: "Preview", icon: Eye },
  ];
  let mode = $state<Mode>("source");

  let activeVault = $state<string | null>(null);
  let activePath = $state<string | null>(null);
  let content = $state("");
  let lastLoaded = $state("");
  let saveTimer: ReturnType<typeof setTimeout> | undefined;

  async function handleOpen(vault: string, path: string) {
    clearTimeout(saveTimer);
    activeVault = vault;
    activePath = path;
    content = await readNote(vault, path);
    lastLoaded = content;
  }

  function handleVaultChange(vault: string | null) {
    clearTimeout(saveTimer);
    activeVault = vault;
    activePath = null;
    content = "";
    lastLoaded = "";
  }

  // Debounced autosave — only when the content actually diverges from what we loaded.
  $effect(() => {
    const c = content;
    const v = activeVault;
    const p = activePath;
    if (!v || !p || c === lastLoaded) return;
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      writeNote(v, p, c);
      lastLoaded = c;
    }, 400);
  });

  // Pull remote edits into the open note. A peer's write (or the blob finishing
  // download) emits vault-changed; re-read the active note. Skip if we have
  // unsaved local edits (content !== lastLoaded) so we don't clobber typing.
  onMount(() => {
    let unlisten: (() => void) | undefined;
    onVaultChanged(async (vaultId) => {
      if (vaultId !== activeVault || !activePath || content !== lastLoaded) return;
      const fresh = await readNote(activeVault, activePath);
      if (fresh !== lastLoaded) {
        content = fresh;
        lastLoaded = fresh;
      }
    }).then((u) => (unlisten = u));
    return () => unlisten?.();
  });
</script>

<div
  class="flex h-screen w-screen flex-col overflow-hidden bg-background text-foreground"
  style="padding-left:env(safe-area-inset-left);padding-right:env(safe-area-inset-right);padding-bottom:env(safe-area-inset-bottom);"
>
  <!-- Top bar -->
  <header
    class="flex min-h-11 shrink-0 items-center justify-between border-b border-border bg-secondary/40 px-3"
    style="padding-top:env(safe-area-inset-top);"
  >
    <span class="truncate text-sm font-medium text-muted-foreground">
      {activePath ?? "notes"}
    </span>

    <div
      class="inline-flex items-center gap-0.5 rounded-md border border-border bg-background p-0.5"
      role="group"
      aria-label="View mode"
    >
      {#each modes as m (m.id)}
        {@const Icon = m.icon}
        <button
          type="button"
          class="flex items-center justify-center rounded-[5px] p-1.5 transition-colors {mode ===
          m.id
            ? 'bg-primary text-primary-foreground'
            : 'text-muted-foreground hover:bg-muted hover:text-foreground'}"
          aria-pressed={mode === m.id}
          aria-label={m.label}
          title={m.label}
          onclick={() => (mode = m.id)}
        >
          <Icon size={16} />
        </button>
      {/each}
    </div>
  </header>

  <!-- Body -->
  <div class="flex min-h-0 flex-1">
    <aside class="w-64 shrink-0 border-r border-border bg-secondary/40">
      <Sidebar
        {activePath}
        onopen={handleOpen}
        onvaultchange={handleVaultChange}
      />
    </aside>

    <main class="min-w-0 flex-1 overflow-auto">
      {#if !activePath}
        <div class="flex h-full items-center justify-center text-sm text-muted-foreground">
          Select or create a note.
        </div>
      {:else if mode === "preview"}
        <Preview value={content} />
      {:else}
        {#key activeVault + ":" + activePath}
          <Editor bind:value={content} />
        {/key}
      {/if}
    </main>
  </div>
</div>
