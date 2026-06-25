<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import type { EditorView } from "@codemirror/view";
  import Editor from "$lib/components/editor/Editor.svelte";
  import Preview from "$lib/components/editor/Preview.svelte";
  import Sidebar from "$lib/components/sidebar/Sidebar.svelte";
  import MarkdownToolbar from "$lib/components/editor/MarkdownToolbar.svelte";
  import SettingsSheet from "$lib/components/SettingsSheet.svelte";
  import Fab from "$lib/components/Fab.svelte";
  import {
    readNote,
    writeNote,
    renamePath,
    deletePath,
    onVaultChanged,
    type TreeNode,
  } from "$lib/vault";
  import { session } from "$lib/session.svelte";
  import { createAndOpenNote, duplicateNote as duplicateNoteFile } from "$lib/notes";
  import { Code, Eye, PanelLeft, NotebookPen, Settings } from "@lucide/svelte";

  type Mode = "source" | "preview";
  let mode = $state<Mode>(session.mode);
  function setMode(m: Mode) {
    mode = m;
    session.mode = m;
  }

  const mobileInit = window.matchMedia("(max-width: 767px)").matches;
  let mobile = $state(mobileInit);
  // macOS desktop uses an Overlay titlebar (traffic lights float over our header,
  // so the window chrome takes the header's color). The header is compacted to the
  // titlebar height and its left edge is inset to clear the lights — except in
  // fullscreen, where macOS hides them and the toggle can sit flush left.
  const isMacDesktop = /Macintosh/.test(navigator.userAgent) && !/Android/.test(navigator.userAgent);
  const chromeIcon = isMacDesktop ? 14 : 16;
  let fullscreen = $state(false);
  onMount(() => {
    if (!isMacDesktop) return;
    const w = getCurrentWindow();
    const sync = () => w.isFullscreen().then((v) => (fullscreen = v));
    sync();
    const un = w.onResized(sync);
    return () => un.then((f) => f());
  });
  // Auto-open the drawer on launch only when no note will be restored. On desktop
  // the sidebar is a persistent panel, so default open there.
  let sidebarOpen = $state(!mobileInit || !session.path);
  function setSidebar(open: boolean) {
    sidebarOpen = open;
  }

  let activeVault = $state<string | null>(null);
  let activePath = $state<string | null>(null);
  let content = $state("");
  let lastLoaded = $state("");
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  let renameTimer: ReturnType<typeof setTimeout> | undefined;
  // Stable per-open id for the editor's {#key}. Lets activePath change under us
  // (H1-driven rename) without remounting the editor and losing the cursor.
  let openToken = $state(0);
  // For a freshly created note: range of the "# Title" text to preselect.
  let pendingSelect = $state<{ from: number; to: number } | null>(null);

  // Editor handle + focus, for the mobile markdown toolbar.
  let editorView = $state<EditorView | undefined>(undefined);
  let editorFocused = $state(false);

  // Settings sheet + the vault tree (for file move/duplicate folder lists).
  let settingsOpen = $state(false);
  let tree = $state<TreeNode[]>([]);
  // Sidebar instance — exposes imperative hooks for global hotkeys.
  let sidebar = $state<Sidebar | undefined>(undefined);

  function* walk(nodes: TreeNode[]): Generator<TreeNode> {
    for (const n of nodes) {
      yield n;
      if (n.children) yield* walk(n.children);
    }
  }
  const folders = $derived([
    { path: "", label: "/" },
    ...[...walk(tree)].filter((n) => n.is_dir).map((n) => ({ path: n.path, label: n.path })),
  ]);
  const currentDir = $derived(
    activePath ? activePath.split("/").slice(0, -1).join("/") : "",
  );

  async function handleOpen(vault: string, path: string, selectTitle = false) {
    clearTimeout(saveTimer);
    clearTimeout(renameTimer);
    activeVault = vault;
    activePath = path;
    session.vault = vault;
    session.path = path;
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
    if (mobile) setSidebar(false);
  }

  function handleVaultChange(vault: string | null) {
    clearTimeout(saveTimer);
    clearTimeout(renameTimer);
    activeVault = vault;
    activePath = null;
    content = "";
    lastLoaded = "";
    pendingSelect = null;
    session.vault = vault;
    session.path = null;
  }

  function closeNote() {
    clearTimeout(saveTimer);
    clearTimeout(renameTimer);
    activePath = null;
    content = "";
    lastLoaded = "";
    pendingSelect = null;
    session.path = null;
  }

  // FAB: new note in the current note's folder (root if none), then edit it.
  async function newNoteHere() {
    if (!activeVault) return;
    const dir = activePath ? activePath.split("/").slice(0, -1).join("/") : "";
    await createAndOpenNote(activeVault, dir, handleOpen);
  }

  // File actions (from the settings sheet).
  async function moveNote(dir: string) {
    if (!activeVault || !activePath) return;
    clearTimeout(saveTimer);
    clearTimeout(renameTimer);
    const base = activePath.split("/").pop()!;
    const to = dir ? `${dir}/${base}` : base;
    await renamePath(activeVault, activePath, to, false);
    handleOpen(activeVault, to);
  }
  // Duplicate the note in the same folder as "X (copy).md" (or "X (copy N).md").
  async function duplicateNote() {
    if (!activeVault || !activePath) return;
    const finalPath = await duplicateNoteFile(activeVault, activePath, tree);
    handleOpen(activeVault, finalPath);
  }
  async function copyContents() {
    try {
      await navigator.clipboard.writeText(content);
    } catch {
      /* clipboard may be unavailable */
    }
  }
  async function deleteNote() {
    if (!activeVault || !activePath) return;
    clearTimeout(saveTimer);
    clearTimeout(renameTimer);
    await deletePath(activeVault, activePath, false);
    closeNote();
  }

  // Autosave. Two debounces:
  //  - 400ms: write content only (no rename) — fast, keeps the DB current.
  //  - 1500ms: once typing settles, allow the H1->filename rename. Splitting
  //    them means typing a title doesn't rename (and emit a sync tombstone) on
  //    every keystroke — only once the title stops changing.
  $effect(() => {
    const c = content;
    const v = activeVault;
    const p = activePath;
    if (!v || !p || c === lastLoaded) return;
    clearTimeout(saveTimer);
    clearTimeout(renameTimer);
    saveTimer = setTimeout(async () => {
      await writeNote(v, p, c, false);
      lastLoaded = c;
    }, 400);
    renameTimer = setTimeout(async () => {
      const finalPath = await writeNote(v, p, c, true);
      lastLoaded = c;
      // The first H1 may have renamed the file. Follow it if we're still on
      // this note (no remount — openToken is unchanged, so the cursor stays).
      if (finalPath !== p && activeVault === v && activePath === p) {
        activePath = finalPath;
        session.path = finalPath;
      }
    }, 1500);
  });

  // Pull remote edits into the open note. A peer's write (or the blob finishing
  // download) emits vault-changed; re-read the active note. Skip if we have
  // unsaved local edits (content !== lastLoaded) so we don't clobber typing.
  onMount(() => {
    let unlisten: (() => void) | undefined;
    // Reactive mobile flag — gates the FAB + markdown toolbar on resize/rotate.
    const mq = window.matchMedia("(max-width: 767px)");
    const onMq = (e: MediaQueryListEvent) => (mobile = e.matches);
    mq.addEventListener("change", onMq);
    window.addEventListener("keydown", onKeydown);
    onVaultChanged(async (vaultId) => {
      if (vaultId !== activeVault || !activePath || content !== lastLoaded) return;
      const fresh = await readNote(activeVault, activePath);
      if (fresh !== lastLoaded) {
        content = fresh;
        lastLoaded = fresh;
      }
    }).then((u) => (unlisten = u));
    return () => {
      mq.removeEventListener("change", onMq);
      window.removeEventListener("keydown", onKeydown);
      unlisten?.();
    };
  });

  // Global hotkeys. Mod = Cmd (macOS) / Ctrl (elsewhere). These complement the
  // editor's own text-formatting shortcuts, which CodeMirror handles internally.
  function onKeydown(e: KeyboardEvent) {
    const mod = e.metaKey || e.ctrlKey;
    if (!mod) return;
    const key = e.key.toLowerCase();
    if (key === "\\") {
      // Toggle the sidebar.
      e.preventDefault();
      setSidebar(!sidebarOpen);
    } else if (key === ",") {
      // Open settings.
      e.preventDefault();
      settingsOpen = true;
    } else if (key === "n" && e.shiftKey) {
      // New folder (at vault root).
      e.preventDefault();
      sidebar?.createFolderHotkey();
    } else if (key === "n") {
      // New note (in the current note's folder).
      e.preventDefault();
      newNoteHere();
    } else if (key === "e" && e.shiftKey) {
      // Toggle source <-> preview (only meaningful with a note open).
      if (activePath) {
        e.preventDefault();
        setMode(mode === "source" ? "preview" : "source");
      }
    }
  }
</script>

<div
  class="flex h-screen w-screen flex-col overflow-hidden bg-background text-foreground"
  style="padding-left:env(safe-area-inset-left);padding-right:env(safe-area-inset-right);padding-bottom:env(safe-area-inset-bottom);"
>
  <!-- Top bar. data-tauri-drag-region lets the window drag by the header, since the
       Overlay titlebar removes the native drag strip; child buttons still click. -->
  <header
    data-tauri-drag-region
    class="flex shrink-0 items-center justify-between border-b border-border bg-secondary/40 {isMacDesktop
      ? 'min-h-0 px-2'
      : 'min-h-12 px-3 pb-2'}"
    style="padding-top:calc(env(safe-area-inset-top) + {isMacDesktop
      ? '0.25rem'
      : '0.5rem'});{isMacDesktop ? 'padding-bottom:0.25rem;' : ''}{isMacDesktop && !fullscreen
      ? 'padding-left:78px;'
      : ''}"
  >
    <div data-tauri-drag-region class="flex min-w-0 items-center gap-2">
      <button
        type="button"
        class="rounded text-muted-foreground hover:bg-muted hover:text-foreground {isMacDesktop
          ? 'p-1'
          : 'p-1.5'}"
        aria-label="Toggle sidebar"
        aria-pressed={sidebarOpen}
        title="Toggle sidebar"
        onclick={() => setSidebar(!sidebarOpen)}
      >
        <PanelLeft size={chromeIcon} />
      </button>
      <!-- Tail path rendering: when the path is too long, the ellipsis collapses
           the *leading* path (left) so the filename stays visible. The container
           is rtl (so overflow/ellipsis lands on the left); an inner `dir="ltr"`
           override keeps the path itself reading left-to-right. -->
      <span data-tauri-drag-region class="path-crumb text-sm font-medium">
        {#if activePath}
          {@const parts = activePath.replace(/\.md$/, "").split("/")}
          <bdo dir="ltr">
            {#each parts as seg, i}
              {#if i > 0}<span class="mx-1.5 text-muted-foreground/40">/</span
                >{/if}<span
                class={i === parts.length - 1
                  ? "text-foreground"
                  : "text-muted-foreground/60"}>{seg}</span
              >
            {/each}
          </bdo>
        {/if}
      </span>
    </div>

    <div data-tauri-drag-region class="flex items-center gap-1.5">
      <!-- Single toggle: click anywhere flips Source<->Preview; active half is lit. -->
      <button
        type="button"
        class="inline-flex items-center gap-0.5 rounded-full border border-border bg-background p-0.5"
        aria-label="Toggle view mode"
        title={mode === "source" ? "Switch to preview" : "Switch to source"}
        onclick={() => setMode(mode === "source" ? "preview" : "source")}
      >
        <span
          class="flex items-center justify-center rounded-full transition-colors {isMacDesktop
            ? 'p-1'
            : 'p-1.5'} {mode === 'source'
            ? 'bg-primary text-primary-foreground'
            : 'text-muted-foreground'}"
        >
          <Code size={chromeIcon} />
        </span>
        <span
          class="flex items-center justify-center rounded-full transition-colors {isMacDesktop
            ? 'p-1'
            : 'p-1.5'} {mode === 'preview'
            ? 'bg-primary text-primary-foreground'
            : 'text-muted-foreground'}"
        >
          <Eye size={chromeIcon} />
        </span>
      </button>
      <button
        type="button"
        class="flex items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-muted hover:text-foreground {isMacDesktop
          ? 'p-1'
          : 'p-2'}"
        aria-label="Settings"
        title="Settings"
        onclick={() => (settingsOpen = true)}
      >
        <Settings size={chromeIcon} />
      </button>
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
        onclick={() => setSidebar(false)}
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
          bind:this={sidebar}
          {activePath}
          onopen={handleOpen}
          onvaultchange={handleVaultChange}
          ontree={(t) => (tree = t)}
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
          <Editor
            bind:value={content}
            selectOnMount={pendingSelect}
            bind:view={editorView}
            bind:focused={editorFocused}
          />
        {/key}
      {/if}
    </main>
  </div>
</div>

<!-- Mobile: floating new-note button (hidden while previewing) -->
{#if mobile && mode !== "preview" && activeVault}
  <Fab onclick={newNoteHere} />
{/if}

<!-- Mobile: markdown toolbar anchored above the soft keyboard while editing -->
{#if mobile && mode === "source" && activePath && editorFocused && editorView}
  <MarkdownToolbar view={editorView} />
{/if}

<SettingsSheet
  bind:open={settingsOpen}
  {activePath}
  {folders}
  {currentDir}
  onmove={moveNote}
  onduplicate={duplicateNote}
  oncopy={copyContents}
  ondelete={deleteNote}
/>

<style>
  /* Left-truncate the breadcrumb: the rtl container puts the ellipsis on the
     left, while the inner `dir="ltr"` <bdo> keeps the path readable. See the
     markup note above. */
  .path-crumb {
    direction: rtl;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    min-width: 0;
  }
</style>
