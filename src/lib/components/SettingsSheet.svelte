<script lang="ts">
  import { fade, fly } from "svelte/transition";
  import { X, Copy, FolderInput, CopyPlus, Trash2 } from "@lucide/svelte";
  import { theme, PALETTES, FONTS, type Mode } from "$lib/theme.svelte";
  import { portal } from "$lib/portal";

  const MODES: { id: Mode; label: string }[] = [
    { id: "system", label: "System" },
    { id: "light", label: "Light" },
    { id: "dark", label: "Dark" },
  ];

  let {
    open = $bindable(false),
    activePath = null,
    folders = [],
    currentDir = "",
    onmove,
    onduplicate,
    oncopy,
    ondelete,
  }: {
    open?: boolean;
    activePath?: string | null;
    folders?: { path: string; label: string }[];
    currentDir?: string;
    onmove: (dir: string) => void;
    onduplicate: () => void;
    oncopy: () => void;
    ondelete: () => void;
  } = $props();

  let movePicker = $state(false);
</script>

{#if open}
  <div
    use:portal
    class="fixed inset-0 z-50 flex items-end justify-center bg-black/50 md:items-center"
    role="presentation"
    transition:fade={{ duration: 150 }}
    onclick={(e) => {
      if (e.target === e.currentTarget) open = false;
    }}
  >
    <div
      class="flex max-h-[80vh] w-full flex-col rounded-t-2xl border border-border bg-popover md:max-w-md md:rounded-2xl"
      style="padding-bottom:env(safe-area-inset-bottom);"
      transition:fly={{ y: 320, duration: 220, opacity: 1 }}
    >
      <div class="flex items-center justify-between border-b border-border px-4 py-3">
        <h2 class="text-base font-semibold">Settings</h2>
        <button
          type="button"
          class="rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground"
          aria-label="Close"
          onclick={() => (open = false)}
        >
          <X class="h-5 w-5" />
        </button>
      </div>

      <div class="min-h-0 flex-1 overflow-auto p-4">
        {#if activePath}
          <!-- File actions (only with a note open) -->
          <p class="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            File
          </p>
          <button
            type="button"
            class="flex w-full items-center gap-2 rounded-md px-2 py-2.5 text-left text-sm hover:bg-muted"
            onclick={() => {
              oncopy();
              open = false;
            }}
          >
            <Copy class="h-4 w-4 shrink-0 text-muted-foreground" />
            <span>Copy contents</span>
          </button>
          <button
            type="button"
            class="flex w-full items-center gap-2 rounded-md px-2 py-2.5 text-left text-sm hover:bg-muted"
            onclick={() => (movePicker = true)}
          >
            <FolderInput class="h-4 w-4 shrink-0 text-muted-foreground" />
            <span>Move to…</span>
          </button>
          <button
            type="button"
            class="flex w-full items-center gap-2 rounded-md px-2 py-2.5 text-left text-sm hover:bg-muted"
            onclick={() => {
              onduplicate();
              open = false;
            }}
          >
            <CopyPlus class="h-4 w-4 shrink-0 text-muted-foreground" />
            <span>Duplicate</span>
          </button>
          <button
            type="button"
            class="flex w-full items-center gap-2 rounded-md px-2 py-2.5 text-left text-sm text-destructive hover:bg-muted"
            onclick={() => {
              ondelete();
              open = false;
            }}
          >
            <Trash2 class="h-4 w-4 shrink-0" />
            <span>Delete note</span>
          </button>

          <div class="my-4 border-t border-border"></div>
        {/if}

        <div class="flex items-center justify-between gap-3 py-1.5">
          <span class="text-sm">Appearance</span>
          <select
            class="min-w-32 rounded-md border border-border bg-background px-2 py-1.5 text-sm"
            value={theme.mode}
            onchange={(e) => (theme.mode = e.currentTarget.value as Mode)}
          >
            {#each MODES as m (m.id)}
              <option value={m.id}>{m.label}</option>
            {/each}
          </select>
        </div>
        <div class="flex items-center justify-between gap-3 py-1.5">
          <span class="text-sm">Theme</span>
          <select
            class="min-w-32 rounded-md border border-border bg-background px-2 py-1.5 text-sm"
            value={theme.palette}
            onchange={(e) => (theme.palette = e.currentTarget.value)}
          >
            {#each PALETTES as p (p.id)}
              <option value={p.id}>{p.name}</option>
            {/each}
          </select>
        </div>
        <div class="flex items-center justify-between gap-3 py-1.5">
          <span class="text-sm">Font</span>
          <select
            class="min-w-32 rounded-md border border-border bg-background px-2 py-1.5 text-sm"
            value={theme.font}
            onchange={(e) => (theme.font = e.currentTarget.value)}
          >
            {#each FONTS as f (f.id)}
              <option value={f.id}>{f.name}</option>
            {/each}
          </select>
        </div>
      </div>
    </div>
  </div>
{/if}

<!-- Move-to folder picker -->
{#if movePicker}
  <div
    use:portal
    class="fixed inset-0 z-[60] flex items-end justify-center bg-black/50 md:items-center"
    role="presentation"
    transition:fade={{ duration: 150 }}
    onclick={(e) => {
      if (e.target === e.currentTarget) movePicker = false;
    }}
  >
    <div
      class="flex max-h-[70vh] w-full flex-col rounded-t-2xl border border-border bg-popover md:max-w-sm md:rounded-2xl"
      style="padding-bottom:env(safe-area-inset-bottom);"
      transition:fly={{ y: 320, duration: 220, opacity: 1 }}
    >
      <div class="flex items-center justify-between border-b border-border px-4 py-3">
        <h2 class="text-base font-semibold">Move to</h2>
        <button
          type="button"
          class="rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground"
          aria-label="Close"
          onclick={() => (movePicker = false)}
        >
          <X class="h-5 w-5" />
        </button>
      </div>
      <div class="min-h-0 flex-1 overflow-auto p-2">
        {#each folders as f (f.path)}
          <button
            type="button"
            class="flex w-full items-center gap-2 rounded-md px-2 py-2.5 text-left text-sm hover:bg-muted disabled:opacity-40"
            disabled={f.path === currentDir}
            onclick={() => {
              onmove(f.path);
              movePicker = false;
              open = false;
            }}
          >
            <FolderInput class="h-4 w-4 shrink-0 text-muted-foreground" />
            <span class="truncate">{f.label}</span>
          </button>
        {/each}
      </div>
    </div>
  </div>
{/if}
