<script lang="ts">
  import { fade, fly } from "svelte/transition";
  import { X, Copy, FolderInput, CopyPlus, Trash2, Pencil, FileDown, FileUp, Archive, ArchiveRestore } from "@lucide/svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { theme, PALETTES, FONTS, type Mode } from "$lib/theme.svelte";
  import { editorSettings } from "$lib/editor-settings.svelte";
  import { liveSync } from "$lib/live-sync.svelte";
  import { checkForUpdateInteractive } from "$lib/updater";
  import { portal } from "$lib/portal";

  // Quick edit is a mobile-only behavior (tap preview → source + keyboard), so
  // only surface its toggle on a touch device — matching the Scan button gate.
  const isMobile = /Android|iPhone|iPad|iPod/i.test(navigator.userAgent);

  const MODES: { id: Mode; label: string }[] = [
    { id: "system", label: "System" },
    { id: "light", label: "Light" },
    { id: "dark", label: "Dark" },
  ];

  // Keyboard shortcuts shown in settings. `keys` are abstract tokens rendered
  // per-platform: Mod = ⌘ on macOS / Ctrl elsewhere, Shift = ⇧ / Shift. Keep
  // this list in sync with App.svelte's onKeydown and Editor.svelte's styleKeymap.
  const isMac = /Macintosh/.test(navigator.userAgent) && !/Android/.test(navigator.userAgent);
  const SHORTCUTS: { keys: string[]; label: string }[] = [
    { keys: ["Mod", "\\"], label: "Toggle sidebar" },
    { keys: ["Mod", ","], label: "Open settings" },
    { keys: ["Mod", "N"], label: "New note" },
    { keys: ["Mod", "Shift", "N"], label: "New folder" },
    { keys: ["Mod", "P"], label: "Toggle source / preview" },
    { keys: ["Mod", "B"], label: "Bold" },
    { keys: ["Mod", "I"], label: "Italic" },
    { keys: ["Mod", "E"], label: "Inline code" },
    { keys: ["Mod", "Shift", "X"], label: "Strikethrough" },
    { keys: ["Mod", "K"], label: "Insert link" },
    { keys: ["Mod", "-"], label: "Toggle checkbox" },
  ];
  function keyLabel(k: string): string {
    if (k === "Mod") return isMac ? "⌘" : "Ctrl";
    if (k === "Shift") return isMac ? "⇧" : "Shift";
    return k.length === 1 ? k.toUpperCase() : k;
  }

  // Show "vN": our release scheme stamps the tag's integer as the semver major
  // (vN -> N.0.0), so the major component is the release number.
  let version = $state("");
  getVersion().then((v) => (version = "v" + v.split(".")[0]));

  // Editor input-assist toggles. `key` indexes into the editorSettings store.
  const EDITOR_TOGGLES: { key: keyof typeof editorSettings; label: string }[] = [
    { key: "autocomplete", label: "Autocomplete" },
    { key: "autocapitalize", label: "Autocapitalize" },
    { key: "autocorrect", label: "Autocorrect" },
    { key: "spellcheck", label: "Spellcheck" },
    { key: "closeBrackets", label: "Autoclose brackets" },
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
    onrename,
    onexportnote,
    onexportvault,
    onimportvault,
    onimportnote,
  }: {
    open?: boolean;
    activePath?: string | null;
    folders?: { path: string; label: string }[];
    currentDir?: string;
    onmove: (dir: string) => void;
    onduplicate: () => void;
    oncopy: () => void;
    ondelete: () => void;
    onrename: () => void;
    onexportnote: () => void;
    onexportvault: () => void;
    onimportvault: () => void;
    onimportnote: () => void;
  } = $props();

  let movePicker = $state(false);

  // Settings is a bottom sheet on mobile (slides up) and a right-side drawer on
  // desktop (slides in from the right). The layout switches via Tailwind md:
  // classes, but the fly transition axis can't be expressed in CSS, so track the
  // breakpoint at runtime to pick y- vs x-slide.
  let isDesktop = $state(false);
  $effect(() => {
    const mq = window.matchMedia("(min-width: 768px)");
    isDesktop = mq.matches;
    const onChange = (e: MediaQueryListEvent) => (isDesktop = e.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  });

  // Swipe-down-to-dismiss on mobile (#46). Driven from the header/grab-handle
  // (not the scrollable body, so it never fights content scroll). The panel
  // tracks the finger downward; release past a threshold closes it, else it
  // snaps back. `sheetY` is the live translateY (px); `sheetDragging` disables
  // the snap transition while the finger is down.
  let sheetY = $state(0);
  let sheetDragging = $state(false);
  let sheetStart: { y: number; id: number } | null = null;
  function onSheetPointerDown(e: PointerEvent) {
    if (isDesktop || e.pointerType === "mouse") return;
    sheetStart = { y: e.clientY, id: e.pointerId };
  }
  function onSheetPointerMove(e: PointerEvent) {
    if (!sheetStart) return;
    const dy = e.clientY - sheetStart.y;
    if (!sheetDragging) {
      if (dy < 8) return; // only engage on a downward drag
      sheetDragging = true;
      (e.currentTarget as HTMLElement).setPointerCapture(sheetStart.id);
    }
    sheetY = Math.max(0, dy);
  }
  function onSheetPointerUp() {
    if (sheetDragging && sheetY > 120) open = false;
    onSheetPointerCancel();
  }
  function onSheetPointerCancel() {
    sheetStart = null;
    sheetDragging = false;
    sheetY = 0;
  }
</script>

{#if open}
  <div
    use:portal
    class="fixed inset-0 z-50 flex items-end justify-center bg-black/50 md:items-stretch md:justify-end"
    role="presentation"
    transition:fade={{ duration: 150 }}
    onclick={(e) => {
      if (e.target === e.currentTarget) open = false;
    }}
  >
    <div
      class="flex max-h-[80vh] w-full flex-col rounded-t-2xl border border-border bg-popover md:h-full md:max-h-none md:max-w-md md:rounded-none md:border-0 md:border-l"
      style="padding-bottom:env(safe-area-inset-bottom);{!isDesktop
        ? `transform:translateY(${sheetY}px);transition:${sheetDragging ? 'none' : 'transform 200ms ease'};`
        : ''}"
      transition:fly={isDesktop
        ? { x: 400, duration: 220, opacity: 1 }
        : { y: 320, duration: 220, opacity: 1 }}
    >
      <!-- Header doubles as the swipe-down-to-dismiss grab handle on mobile (#46);
           touch-action:none so the drag isn't stolen by the browser as a scroll. -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="flex flex-col border-b border-border"
        style="touch-action:none;"
        onpointerdown={onSheetPointerDown}
        onpointermove={onSheetPointerMove}
        onpointerup={onSheetPointerUp}
        onpointercancel={onSheetPointerCancel}
      >
        <div class="mx-auto mt-2 h-1 w-9 rounded-full bg-muted-foreground/30 md:hidden"></div>
        <div class="flex items-center justify-between px-4 py-3">
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
              onrename();
              open = false;
            }}
          >
            <Pencil class="h-4 w-4 shrink-0 text-muted-foreground" />
            <span>Rename</span>
          </button>
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
            class="flex w-full items-center gap-2 rounded-md px-2 py-2.5 text-left text-sm hover:bg-muted"
            onclick={() => {
              onexportnote();
              open = false;
            }}
          >
            <FileDown class="h-4 w-4 shrink-0 text-muted-foreground" />
            <span>Export as Markdown</span>
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

        <p class="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          Data
        </p>
        <button
          type="button"
          class="flex w-full items-center gap-2 rounded-md px-2 py-2.5 text-left text-sm hover:bg-muted"
          onclick={() => {
            onimportnote();
            open = false;
          }}
        >
          <FileUp class="h-4 w-4 shrink-0 text-muted-foreground" />
          <span>Import note (.md)</span>
        </button>
        <button
          type="button"
          class="flex w-full items-center gap-2 rounded-md px-2 py-2.5 text-left text-sm hover:bg-muted"
          onclick={() => {
            onexportvault();
            open = false;
          }}
        >
          <Archive class="h-4 w-4 shrink-0 text-muted-foreground" />
          <span>Export vault (.zip)</span>
        </button>
        <button
          type="button"
          class="flex w-full items-center gap-2 rounded-md px-2 py-2.5 text-left text-sm hover:bg-muted"
          onclick={() => {
            onimportvault();
            open = false;
          }}
        >
          <ArchiveRestore class="h-4 w-4 shrink-0 text-muted-foreground" />
          <span>Import vault (.zip)</span>
        </button>

        <div class="my-4 border-t border-border"></div>

        <p class="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          Appearance
        </p>
        <div class="flex items-center justify-between gap-3 py-1.5">
          <span class="text-sm">Mode</span>
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

        <div class="my-4 border-t border-border"></div>
        <p class="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          Editor
        </p>
        {#each EDITOR_TOGGLES as t (t.key)}
          <label class="flex cursor-pointer items-center justify-between gap-3 py-1.5">
            <span class="text-sm">{t.label}</span>
            <input
              type="checkbox"
              class="h-4 w-4 accent-primary"
              checked={editorSettings[t.key]}
              onchange={(e) => (editorSettings[t.key] = e.currentTarget.checked)}
            />
          </label>
        {/each}
        {#if isMobile}
          <label class="flex cursor-pointer items-center justify-between gap-3 py-1.5">
            <span class="flex flex-col">
              <span class="text-sm">Quick edit</span>
              <span class="text-xs text-muted-foreground">
                Tap a preview to edit; hide the keyboard to return.
              </span>
            </span>
            <input
              type="checkbox"
              class="h-4 w-4 shrink-0 accent-primary"
              checked={editorSettings.quickEdit}
              onchange={(e) => (editorSettings.quickEdit = e.currentTarget.checked)}
            />
          </label>
        {/if}

        {#if !isMobile}
          <div class="my-4 border-t border-border"></div>
          <p class="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            Sync
          </p>
          <label class="flex cursor-pointer items-center justify-between gap-3 py-1.5">
            <span class="flex flex-col">
              <span class="text-sm">Background sync</span>
              <span class="text-xs text-muted-foreground">
                Keep this Mac syncing as a hub when the window is closed (stays in
                the menu bar) and start it at login.
              </span>
            </span>
            <input
              type="checkbox"
              class="h-4 w-4 shrink-0 accent-primary"
              checked={liveSync.enabled}
              onchange={(e) => (liveSync.enabled = e.currentTarget.checked)}
            />
          </label>
        {/if}

        <div class="my-4 border-t border-border"></div>
        <p class="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          Keyboard shortcuts
        </p>
        {#each SHORTCUTS as s (s.label)}
          <div class="flex items-center justify-between gap-3 py-1.5">
            <span class="text-sm">{s.label}</span>
            <span class="flex items-center gap-1">
              {#each s.keys as k (k)}
                <kbd
                  class="min-w-5 rounded border border-border bg-muted px-1.5 py-0.5 text-center text-xs text-muted-foreground"
                >
                  {keyLabel(k)}
                </kbd>
              {/each}
            </span>
          </div>
        {/each}

        <div class="my-4 border-t border-border"></div>
        <div class="flex items-center justify-between gap-3">
          <p class="text-xs text-muted-foreground">
            {version}
          </p>
          {#if !isMobile}
            <button
              type="button"
              class="rounded border border-border px-2 py-1 text-xs text-muted-foreground hover:bg-muted"
              onclick={() => checkForUpdateInteractive()}
            >
              Check for updates
            </button>
          {/if}
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
