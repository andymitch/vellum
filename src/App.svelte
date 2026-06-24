<script lang="ts">
  import { onMount } from "svelte";
  import Editor from "$lib/components/editor/Editor.svelte";
  import Preview from "$lib/components/editor/Preview.svelte";
  import Sidebar from "$lib/components/sidebar/Sidebar.svelte";
  import { readNote, writeNote, onVaultChanged } from "$lib/vault";
  import { Code, Eye, PanelLeft, NotebookPen, type Icon as IconType } from "@lucide/svelte";

  type Mode = "source" | "preview";
  const modes: { id: Mode; label: string; icon: typeof IconType }[] = [
    { id: "source", label: "Source", icon: Code },
    { id: "preview", label: "Preview", icon: Eye },
  ];
  let mode = $state<Mode>("source");

  let sidebarOpen = $state(true);
  const isMobile = () => window.matchMedia("(max-width: 767px)").matches;

  let activeVault = $state<string | null>(null);
  let activePath = $state<string | null>(null);
  let content = $state("");
  let lastLoaded = $state("");
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  // Stable per-open id for the editor's {#key}. Lets activePath change under us
  // (H1-driven rename) without remounting the editor and losing the cursor.
  let openToken = $state(0);
  // For a freshly created note: range of the "# Title" text to preselect.
  let pendingSelect = $state<{ from: number; to: number } | null>(null);

  async function handleOpen(vault: string, path: string, selectTitle = false) {
    clearTimeout(saveTimer);
    activeVault = vault;
    activePath = path;
    content = await readNote(vault, path);
    lastLoaded = content;
    // Preselect the H1 title (the text after "# " on the first line) so a new
    // note can be renamed by just typing.
    if (selectTitle && content.startsWith("# ")) {
      const nl = content.indexOf("\n");
      pendingSelect = { from: 2, to: nl === -1 ? content.length : nl };
    } else {
      pendingSelect = null;
    }
    openToken++;
    if (isMobile()) sidebarOpen = false;
  }

  function handleVaultChange(vault: string | null) {
    clearTimeout(saveTimer);
    activeVault = vault;
    activePath = null;
    content = "";
    lastLoaded = "";
    pendingSelect = null;
  }

  // Debounced autosave — only when the content actually diverges from what we loaded.
  $effect(() => {
    const c = content;
    const v = activeVault;
    const p = activePath;
    if (!v || !p || c === lastLoaded) return;
    clearTimeout(saveTimer);
    saveTimer = setTimeout(async () => {
      const finalPath = await writeNote(v, p, c);
      lastLoaded = c;
      // The first H1 may have renamed the file. Follow it if we're still on
      // this note (no remount — openToken is unchanged, so the cursor stays).
      if (finalPath !== p && activeVault === v && activePath === p) {
        activePath = finalPath;
      }
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
    class="flex min-h-12 shrink-0 items-center justify-between border-b border-border bg-secondary/40 px-3 pb-2"
    style="padding-top:calc(env(safe-area-inset-top) + 0.5rem);"
  >
    <div class="flex min-w-0 items-center gap-2">
      <button
        type="button"
        class="rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground"
        aria-label="Toggle sidebar"
        aria-pressed={sidebarOpen}
        title="Toggle sidebar"
        onclick={() => (sidebarOpen = !sidebarOpen)}
      >
        <PanelLeft size={16} />
      </button>
      <span class="truncate text-sm font-medium">
        {#if activePath}
          {@const parts = activePath.replace(/\.md$/, "").split("/")}
          {#each parts as seg, i}
            {#if i > 0}<span class="mx-1.5 text-muted-foreground/40">/</span>{/if}
            <span
              class={i === parts.length - 1
                ? "text-foreground"
                : "text-muted-foreground/60"}>{seg}</span
            >
          {/each}
        {:else}
          <span class="text-muted-foreground">notes</span>
        {/if}
      </span>
    </div>

    <div
      class="inline-flex items-center gap-0.5 rounded-full border border-border bg-background p-0.5"
      role="group"
      aria-label="View mode"
    >
      {#each modes as m (m.id)}
        {@const Icon = m.icon}
        <button
          type="button"
          class="flex items-center justify-center rounded-full p-1.5 transition-colors {mode ===
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
  <div class="relative flex min-h-0 flex-1">
    <!-- Mobile backdrop -->
    {#if sidebarOpen}
      <button
        type="button"
        class="fixed inset-0 z-20 bg-black/50 md:hidden"
        aria-label="Close sidebar"
        onclick={() => (sidebarOpen = false)}
      ></button>
    {/if}

    <!-- Collapsible on desktop, drawer on mobile -->
    <aside
      class="z-30 shrink-0 overflow-hidden border-border bg-secondary transition-all duration-200 ease-out fixed inset-y-0 left-0 w-64 md:static md:z-auto md:bg-secondary/40 {sidebarOpen
        ? 'translate-x-0 border-r md:w-64'
        : '-translate-x-full border-r-0 md:translate-x-0 md:w-0'}"
    >
      <!-- Drawer is fixed on mobile, so it escapes the root's safe-area padding;
           re-apply top/bottom insets here so content clears the status/nav bars.
           Insets are 0 on desktop, so this is a no-op there. -->
      <div
        class="h-full w-64"
        style="padding-top:env(safe-area-inset-top);padding-bottom:env(safe-area-inset-bottom);"
      >
        <Sidebar
          {activePath}
          onopen={handleOpen}
          onvaultchange={handleVaultChange}
        />
      </div>
    </aside>

    <main class="min-w-0 flex-1 overflow-auto">
      {#if !activePath}
        <div class="flex h-full flex-col items-center justify-center gap-3 text-muted-foreground">
          <NotebookPen size={40} class="opacity-30" />
          <p class="text-sm">Select or create a note.</p>
        </div>
      {:else if mode === "preview"}
        <Preview value={content} />
      {:else}
        {#key openToken}
          <Editor bind:value={content} selectOnMount={pendingSelect} />
        {/key}
      {/if}
    </main>
  </div>
</div>
