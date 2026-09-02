<script lang="ts">
  import { fade, fly, slide } from "svelte/transition";
  import { X, Copy, FolderInput, FolderPlus, FolderOpen, CopyPlus, Trash2, Pencil, FileDown, FileUp, Archive, ArchiveRestore, BookOpen, ChevronRight, Newspaper, Keyboard, Share2, Mail } from "@lucide/svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { theme, PALETTES, FONTS, type Mode } from "$lib/theme.svelte";
  import { editorSettings } from "$lib/editor-settings.svelte";
  import { liveSync } from "$lib/live-sync.svelte";
  import { mcp } from "$lib/mcp.svelte";
  import { linkFolders } from "$lib/link-folders.svelte";
  import { listVaults, type VaultInfo } from "$lib/vault";
  import { betaChannel } from "$lib/beta-channel.svelte";
  import {
    checkForUpdateInteractive,
    checkForUpdateMobile,
    formatVersion,
    fetchChannelRelease,
    type LatestRelease,
  } from "$lib/updater";
  import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
  import { marked } from "marked";
  import { portal } from "$lib/portal";

  // Transient "Copied" confirmation on the MCP connect command.
  let copiedMcp = $state(false);
  async function copyMcpCommand() {
    if (!mcp.command) return;
    try {
      await navigator.clipboard.writeText(mcp.command);
      copiedMcp = true;
      setTimeout(() => (copiedMcp = false), 1500);
    } catch {
      /* clipboard may be unavailable */
    }
  }

  // Linked folders (#219). The "Add" form's vault list is fetched lazily on
  // first open, same reasoning as "What's new"'s lazy release-notes fetch.
  let addLinkOpen = $state(false);
  let linkVaults = $state<VaultInfo[]>([]);
  let newLinkVault = $state("");
  let newLinkFolder = $state("");
  let addLinkError = $state("");
  async function openAddLink() {
    addLinkOpen = true;
    addLinkError = "";
    try {
      linkVaults = await listVaults();
      if (!newLinkVault && linkVaults.length) newLinkVault = linkVaults[0].id;
    } catch {
      linkVaults = [];
    }
  }
  async function submitAddLink() {
    if (!newLinkVault) return;
    addLinkError = "";
    try {
      await linkFolders.add(newLinkVault, newLinkFolder);
      newLinkFolder = "";
      addLinkOpen = false;
    } catch (e) {
      addLinkError = e instanceof Error ? e.message : String(e);
    }
  }

  let copiedLinkId = $state<string | null>(null);
  async function copyLinkPath(path: string, id: string) {
    try {
      await navigator.clipboard.writeText(path);
      copiedLinkId = id;
      setTimeout(() => (copiedLinkId = null), 1500);
    } catch {
      /* clipboard may be unavailable */
    }
  }

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
    { keys: ["Mod", "F"], label: "Search notes" },
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

  // Markdown cheatsheet (#155), grouped, shown as a collapsible section in
  // Settings. `syntax` renders verbatim in a <code> block; a `\n` in it makes a
  // multi-line example. Covers GFM plus Vellum's own wiki links (`[[...]]`,
  // resolved in Preview.svelte).
  const CHEATS: { group: string; rows: { syntax: string; label: string }[] }[] = [
    {
      group: "Text",
      rows: [
        { syntax: "**bold**", label: "Bold" },
        { syntax: "*italic*", label: "Italic" },
        { syntax: "~~strikethrough~~", label: "Strikethrough" },
        { syntax: "`inline code`", label: "Inline code" },
      ],
    },
    {
      group: "Headings",
      rows: [
        { syntax: "# Heading 1", label: "Largest" },
        { syntax: "## Heading 2", label: "" },
        { syntax: "###### Heading 6", label: "Smallest" },
      ],
    },
    {
      group: "Lists",
      rows: [
        { syntax: "- item\n- item", label: "Bulleted" },
        { syntax: "1. item\n2. item", label: "Numbered" },
        { syntax: "- [ ] to do\n- [x] done", label: "Task list" },
      ],
    },
    {
      group: "Links",
      rows: [
        { syntax: "[label](https://url)", label: "External link" },
        { syntax: "![alt](image-url)", label: "Image" },
        { syntax: "[[Note title]]", label: "Link another note" },
        { syntax: "[[Note title|label]]", label: "Linked note, custom label" },
        { syntax: "[[Note#Heading]]", label: "Link a heading in a note" },
        { syntax: "[[#Heading]]", label: "Link a heading in this note" },
      ],
    },
    {
      group: "Blocks",
      rows: [
        { syntax: "> quoted text", label: "Blockquote" },
        { syntax: "```lang\ncode\n```", label: "Fenced code block" },
        { syntax: "```mermaid\ngraph TD; A-->B\n```", label: "Mermaid diagram" },
        { syntax: "| a | b |\n| - | - |\n| 1 | 2 |", label: "Table" },
        { syntax: "---", label: "Horizontal rule" },
      ],
    },
  ];
  let cheatOpen = $state(false);
  let shortcutsOpen = $state(false);

  // "What's new" (#144): lazily fetch release notes on first expand and render
  // them as markdown. "loading"/"error" are sentinel states.
  //
  // Channel-aware since #210: a beta tester was previously shown the notes for
  // the newest STABLE release, which is older than the build they are running.
  let whatsNewOpen = $state(false);
  let whatsNew = $state<LatestRelease | "loading" | "error" | null>(null);
  async function toggleWhatsNew() {
    whatsNewOpen = !whatsNewOpen;
    if (whatsNewOpen && whatsNew === null) {
      whatsNew = "loading";
      whatsNew = (await fetchChannelRelease()) ?? "error";
    }
  }
  // Release notes are trusted (our own generated changelog), so render markdown
  // directly. Links would otherwise navigate the whole webview — intercept and
  // open them in the OS browser instead.
  function onNotesClick(e: MouseEvent) {
    const a = (e.target as HTMLElement | null)?.closest("a");
    if (!a) return;
    e.preventDefault();
    const href = a.getAttribute("href");
    if (href) openUrl(href);
  }

  // Drop trailing-zero components (5.0.0 -> v5, 5.1.0 -> v5.1, 5.0.1 -> v5.0.1).
  let version = $state("");
  getVersion().then((v) => (version = formatVersion(v)));

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
    onsharenote,
    onemailnote,
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
    onsharenote: () => void;
    onemailnote: () => void;
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
            class="flex w-full items-center gap-2 rounded-md px-2 py-2.5 text-left text-sm hover:bg-muted"
            onclick={() => {
              onsharenote();
              open = false;
            }}
          >
            <Share2 class="h-4 w-4 shrink-0 text-muted-foreground" />
            <span>Share…</span>
          </button>
          <button
            type="button"
            class="flex w-full items-center gap-2 rounded-md px-2 py-2.5 text-left text-sm hover:bg-muted"
            onclick={() => {
              onemailnote();
              open = false;
            }}
          >
            <Mail class="h-4 w-4 shrink-0 text-muted-foreground" />
            <span>Email…</span>
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
          <label class="flex cursor-pointer items-center justify-between gap-3 py-1.5">
            <span class="flex flex-col">
              <span class="text-sm">Journal: Return adds a line break</span>
              <span class="text-xs text-muted-foreground">
                Shift+Return finishes the cell instead. Off: Return finishes it, and
                Shift+Return adds the line break.
              </span>
            </span>
            <input
              type="checkbox"
              class="h-4 w-4 shrink-0 accent-primary"
              checked={editorSettings.journalReturnNewline}
              onchange={(e) => (editorSettings.journalReturnNewline = e.currentTarget.checked)}
            />
          </label>
        {/if}
        <label class="flex cursor-pointer items-center justify-between gap-3 py-1.5">
          <span class="flex flex-col">
            <span class="text-sm">Link previews</span>
            <span class="text-xs text-muted-foreground">
              Show a card for a link alone on its line. Cards for web links fetch that
              page — the only request Vellum makes.
            </span>
          </span>
          <input
            type="checkbox"
            class="h-4 w-4 shrink-0 accent-primary"
            checked={editorSettings.linkPreviews}
            onchange={(e) => (editorSettings.linkPreviews = e.currentTarget.checked)}
          />
        </label>

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

          <label class="flex cursor-pointer items-center justify-between gap-3 py-1.5">
            <span class="flex flex-col">
              <span class="text-sm">Beta updates</span>
              <span class="text-xs text-muted-foreground">
                Receive pre-release builds. While you're on a beta the stable
                channel reports nothing to install, since your version is already
                ahead of it — you'll move back onto stable when it overtakes you.
              </span>
            </span>
            <input
              type="checkbox"
              class="h-4 w-4 shrink-0 accent-primary"
              checked={betaChannel.enabled}
              onchange={(e) => (betaChannel.enabled = e.currentTarget.checked)}
            />
          </label>

          <!-- Agent access (#164). Desktop only: the MCP server is hosted by
               this app on loopback, and a phone has nothing that could reach it. -->
          <div class="my-4 border-t border-border"></div>
          <p class="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            Agents
          </p>
          <label class="flex cursor-pointer items-center justify-between gap-3 py-1.5">
            <span class="flex flex-col">
              <span class="text-sm">Agent access (MCP)</span>
              <span class="text-xs text-muted-foreground">
                Let Claude Code and other MCP clients read and write your notes.
                Listens on this Mac only (127.0.0.1) and requires the token below.
              </span>
            </span>
            <input
              type="checkbox"
              class="h-4 w-4 shrink-0 accent-primary"
              disabled={mcp.busy}
              checked={mcp.enabled}
              onchange={(e) => mcp.toggle(e.currentTarget.checked)}
            />
          </label>
          {#if mcp.enabled && mcp.command}
            <div class="mt-1 flex items-center justify-between gap-3 py-1.5">
              <code class="truncate text-xs text-muted-foreground">{mcp.url}</code>
              <button
                class="flex shrink-0 items-center gap-1 rounded border border-border px-2 py-1 text-xs hover:bg-muted"
                onclick={() => copyMcpCommand()}
              >
                <Copy class="h-3 w-3" />
                {copiedMcp ? "Copied" : "Copy connect command"}
              </button>
            </div>
          {/if}

          <!-- Linked folders (#219). Desktop only: the mirror is watched by
               this app process, and a phone can't watch an arbitrary directory. -->
          <div class="my-4 border-t border-border"></div>
          <p class="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            Linked folders
          </p>
          {#each linkFolders.all as link (link.id)}
            <div class="flex flex-col gap-1 py-1.5">
              <div class="flex items-center justify-between gap-3">
                <span class="flex flex-col overflow-hidden">
                  <span class="truncate text-sm">
                    {link.vault_name}{link.folder ? `/${link.folder}` : ""}
                  </span>
                  <code class="truncate text-xs text-muted-foreground">{link.path}</code>
                </span>
                <input
                  type="checkbox"
                  class="h-4 w-4 shrink-0 accent-primary"
                  disabled={linkFolders.busy}
                  checked={link.enabled}
                  onchange={(e) => linkFolders.toggle(link.id, e.currentTarget.checked)}
                />
              </div>
              <div class="flex items-center gap-2">
                <button
                  type="button"
                  class="flex items-center gap-1 rounded border border-border px-2 py-1 text-xs hover:bg-muted"
                  onclick={() => copyLinkPath(link.path, link.id)}
                >
                  <Copy class="h-3 w-3" />
                  {copiedLinkId === link.id ? "Copied" : "Copy path"}
                </button>
                <button
                  type="button"
                  class="flex items-center gap-1 rounded border border-border px-2 py-1 text-xs hover:bg-muted"
                  onclick={() => revealItemInDir(link.path)}
                >
                  <FolderOpen class="h-3 w-3" />
                  Reveal
                </button>
                <button
                  type="button"
                  class="ml-auto flex items-center gap-1 rounded border border-border px-2 py-1 text-xs text-destructive hover:bg-muted"
                  disabled={linkFolders.busy}
                  onclick={() => linkFolders.remove(link.id)}
                >
                  <Trash2 class="h-3 w-3" />
                  Remove
                </button>
              </div>
            </div>
          {/each}
          {#if !addLinkOpen}
            <button
              type="button"
              class="flex w-full items-center gap-2 rounded-md px-2 py-2.5 text-left text-sm hover:bg-muted"
              onclick={openAddLink}
            >
              <FolderPlus class="h-4 w-4 shrink-0 text-muted-foreground" />
              <span>Add linked folder…</span>
            </button>
          {:else}
            <div class="mt-1 flex flex-col gap-2 rounded-md border border-border p-2" transition:slide={{ duration: 150 }}>
              <span class="text-xs text-muted-foreground">
                Mirrors to a folder under <code>~/.vellum/local/</code> that you can
                open, or add to an editor's project (e.g. Zed's "add folder to
                project") alongside your code.
              </span>
              <div class="flex items-center gap-2">
                <span class="w-14 shrink-0 text-xs text-muted-foreground">Vault</span>
                <select
                  class="min-w-0 flex-1 rounded-md border border-border bg-background px-2 py-1.5 text-sm"
                  bind:value={newLinkVault}
                >
                  {#each linkVaults as v (v.id)}
                    <option value={v.id}>{v.name}</option>
                  {/each}
                </select>
              </div>
              <div class="flex items-center gap-2">
                <span class="w-14 shrink-0 text-xs text-muted-foreground">Folder</span>
                <input
                  type="text"
                  placeholder="whole vault"
                  class="min-w-0 flex-1 rounded-md border border-border bg-background px-2 py-1.5 text-sm"
                  bind:value={newLinkFolder}
                />
              </div>
              {#if addLinkError}
                <p class="text-xs text-destructive">{addLinkError}</p>
              {/if}
              <div class="flex justify-end gap-2">
                <button
                  type="button"
                  class="rounded border border-border px-2 py-1 text-xs hover:bg-muted"
                  onclick={() => (addLinkOpen = false)}
                >
                  Cancel
                </button>
                <button
                  type="button"
                  class="rounded border border-border bg-primary px-2 py-1 text-xs text-primary-foreground hover:opacity-90"
                  disabled={linkFolders.busy || !newLinkVault}
                  onclick={submitAddLink}
                >
                  Add
                </button>
              </div>
            </div>
          {/if}
        {/if}

        <div class="my-4 border-t border-border"></div>
        <p class="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          Help
        </p>
        <button
          type="button"
          class="flex w-full items-center gap-2 rounded-md px-2 py-2.5 text-left text-sm hover:bg-muted"
          aria-expanded={cheatOpen}
          onclick={() => (cheatOpen = !cheatOpen)}
        >
          <BookOpen class="h-4 w-4 shrink-0 text-muted-foreground" />
          <span>Markdown cheatsheet</span>
          <ChevronRight
            class="ml-auto h-4 w-4 shrink-0 text-muted-foreground transition-transform {cheatOpen
              ? 'rotate-90'
              : ''}"
          />
        </button>
        {#if cheatOpen}
          <div class="mt-1 px-2 pb-1" transition:slide={{ duration: 150 }}>
            {#each CHEATS as section (section.group)}
              <p class="mb-1.5 mt-3 text-xs font-semibold uppercase tracking-wide text-muted-foreground first:mt-0">
                {section.group}
              </p>
              <div class="flex flex-col gap-1.5">
                {#each section.rows as row (row.syntax)}
                  <div class="flex items-center justify-between gap-3">
                    <code
                      class="whitespace-pre rounded bg-muted px-1.5 py-1 text-xs text-muted-foreground"
                      >{row.syntax}</code
                    >
                    {#if row.label}
                      <span class="shrink-0 text-right text-xs text-muted-foreground/70">
                        {row.label}
                      </span>
                    {/if}
                  </div>
                {/each}
              </div>
            {/each}
          </div>
        {/if}
        <button
          type="button"
          class="flex w-full items-center gap-2 rounded-md px-2 py-2.5 text-left text-sm hover:bg-muted"
          aria-expanded={whatsNewOpen}
          onclick={toggleWhatsNew}
        >
          <Newspaper class="h-4 w-4 shrink-0 text-muted-foreground" />
          <span>What's new</span>
          <ChevronRight
            class="ml-auto h-4 w-4 shrink-0 text-muted-foreground transition-transform {whatsNewOpen
              ? 'rotate-90'
              : ''}"
          />
        </button>
        {#if whatsNewOpen}
          <div class="mt-1 px-2 pb-1" transition:slide={{ duration: 150 }}>
            {#if whatsNew === "loading"}
              <p class="text-xs text-muted-foreground">Loading…</p>
            {:else if whatsNew === "error" || !whatsNew}
              <p class="text-xs text-muted-foreground">
                Couldn't load the changelog.
                <button
                  type="button"
                  class="underline hover:text-foreground"
                  onclick={() => openUrl("https://github.com/andymitch/vellum/releases/latest")}
                  >View on GitHub</button
                >
              </p>
            {:else}
              <p class="mb-1 flex items-center gap-1.5 text-xs font-semibold text-muted-foreground">
                <span>{whatsNew.tag}</span>
                {#if whatsNew.prerelease}
                  <span
                    class="rounded-full bg-muted px-1.5 py-0.5 text-[0.65rem] font-medium uppercase tracking-wide"
                    >beta</span
                  >
                {/if}
              </p>
              <!-- eslint-disable-next-line svelte/no-at-html-tags -->
              <div class="release-notes text-sm text-muted-foreground" onclick={onNotesClick} role="presentation">
                {@html marked.parse(whatsNew.notes)}
              </div>
            {/if}
          </div>
        {/if}

        <button
          type="button"
          class="flex w-full items-center gap-2 rounded-md px-2 py-2.5 text-left text-sm hover:bg-muted"
          aria-expanded={shortcutsOpen}
          onclick={() => (shortcutsOpen = !shortcutsOpen)}
        >
          <Keyboard class="h-4 w-4 shrink-0 text-muted-foreground" />
          <span>Keyboard shortcuts</span>
          <ChevronRight
            class="ml-auto h-4 w-4 shrink-0 text-muted-foreground transition-transform {shortcutsOpen
              ? 'rotate-90'
              : ''}"
          />
        </button>
        {#if shortcutsOpen}
          <div class="mt-1 px-2 pb-1" transition:slide={{ duration: 150 }}>
            {#each SHORTCUTS as s (s.label)}
              <div class="flex items-center justify-between gap-3 py-1.5">
                <span class="text-sm text-muted-foreground">{s.label}</span>
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
          </div>
        {/if}

        <div class="my-4 border-t border-border"></div>
        <div class="flex items-center justify-between gap-3">
          <p class="text-xs text-muted-foreground">
            {version}
          </p>
          <button
            type="button"
            class="rounded border border-border px-2 py-1 text-xs text-muted-foreground hover:bg-muted"
            onclick={() => (isMobile ? checkForUpdateMobile(true) : checkForUpdateInteractive())}
          >
            Check for updates
          </button>
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

<style>
  /* Release notes (#144) are injected via {@html}, so reach them with :global.
     Keep it compact — this is a small panel, not the note preview. */
  .release-notes :global(h1),
  .release-notes :global(h2),
  .release-notes :global(h3) {
    margin: 0.75rem 0 0.25rem;
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .release-notes :global(ul) {
    margin: 0.25rem 0;
    padding-left: 1.1rem;
    list-style: disc;
  }
  .release-notes :global(li) {
    margin: 0.15rem 0;
  }
  .release-notes :global(p) {
    margin: 0.35rem 0;
  }
  .release-notes :global(a) {
    color: var(--primary);
    text-decoration: underline;
  }
  .release-notes :global(code) {
    border-radius: 4px;
    background: var(--muted);
    padding: 0.05em 0.3em;
    font-size: 0.85em;
  }
</style>
