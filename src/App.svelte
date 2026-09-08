<script lang="ts">
  import { onMount, tick } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import NotePane from "$lib/components/editor/NotePane.svelte";
  import Splitter from "$lib/components/Splitter.svelte";
  import Sidebar from "$lib/components/sidebar/Sidebar.svelte";
  import TabStrip from "$lib/components/TabStrip.svelte";
  import SettingsSheet from "$lib/components/SettingsSheet.svelte";
  import SearchPalette from "$lib/components/SearchPalette.svelte";
  import { checkForUpdate, checkForUpdateMobile } from "$lib/updater";
  import Fab from "$lib/components/Fab.svelte";
  import {
    renamePath,
    deletePath,
    shareNote,
    onBackgroundSyncChanged,
    type TreeNode,
  } from "$lib/vault";
  import { isAndroidApp, isMacApp } from "$lib/platform";
  import { softKeyboard } from "$lib/soft-keyboard";
  import { session } from "$lib/session.svelte";
  import { duplicateNote as duplicateNoteFile } from "$lib/notes";
  import {
    exportVaultZip,
    importVaultZip,
    exportNoteMd,
    importNoteMd,
    emailNote,
  } from "$lib/transfer";
  import { initLiveSync, applyLiveSyncFromBackend } from "$lib/live-sync.svelte";
  import { initMcp } from "$lib/mcp.svelte";
  import { initLinkFolders } from "$lib/link-folders.svelte";
  import { noteTypeInfo, type NoteType } from "$lib/note-type";
  import {
    Code,
    Eye,
    PanelLeft,
    NotebookPen,
    Settings,
    Search,
    X,
  } from "@lucide/svelte";

  type Mode = "source" | "preview";
  // The view mode belongs to the active tab (#169), so it lives in the session
  // store rather than here — switching tabs switches mode with it.
  const mode = $derived<Mode>(session.mode);
  // Auto-hide the editor chrome (top bar + FAB) on scroll-down, reveal on
  // scroll-up, so the reading/writing surface is unobstructed on small screens
  // while the controls stay one gesture away (#85). The decision is the open
  // pane's — it watches its own scroller — and this is where it is applied.
  //
  // The chrome floats over that scroller, which carries a *constant* top
  // padding of the header height (see markup). Hiding it is a pure
  // transform+fade that never changes layout, so it can't nudge scrollTop: the
  // padding just scrolls off the top like any other content, and back into view
  // (with the header flying back in) on scroll-up (#100).
  let chromeHidden = $state(false);
  // Seed near the real height (min-h-12 + pb-2) so the mobile body's
  // padding-top:headerH doesn't jump on the first frame before offsetHeight
  // binds (#100). The bind corrects it (incl. safe-area) a frame later.
  let headerH = $state(56);
  // When the user last *moved* a finger or a wheel. Only a scroll that follows
  // close behind real input is allowed to move the chrome (#248): CodeMirror
  // scrolls the caret into view when you type onto a new line, and reading that
  // few-pixel shift as a scroll-down slid the header away mid-sentence.
  //
  // Movement rather than contact, for two reasons. A tap is not a scroll: a
  // quick-edit tap is followed by the editor mounting and scrolling itself, and
  // that must not count. And movement keeps arriving for as long as a drag
  // lasts, so there is no finger-down flag that can be left set by a touchend
  // we never saw — the window simply lapses.
  let lastScrollGestureAt = 0;
  // Long enough to cover the fling after the finger lifts.
  const GESTURE_GRACE_MS = 350;
  const userScrolling = () => Date.now() - lastScrollGestureAt < GESTURE_GRACE_MS;
  const mobileInit = window.matchMedia("(max-width: 767px)").matches;
  let mobile = $state(mobileInit);
  // The installed macOS app uses an Overlay titlebar (traffic lights float over our
  // header, so the window chrome takes the header's color). The header is compacted
  // to the titlebar height and its left edge is inset to clear the lights — except
  // in fullscreen, where macOS hides them and the toggle can sit flush left. Safari
  // on a Mac has no titlebar to dodge, hence isMacApp rather than a UA test.
  const chromeIcon = isMacApp ? 14 : 16;
  let fullscreen = $state(false);
  // Check for an app update on launch: desktop via the Tauri updater, Android via
  // the GitHub releases check (#145). Both silently no-op if up to date/offline.
  // The web app updates itself — the service worker fetches a new build in the
  // background and it applies on the next load (#221).
  onMount(() => {
    if (isMacApp) checkForUpdate();
    else if (isAndroidApp) checkForUpdateMobile();
  });
  // Re-apply the Background sync setting on launch (re-arm the hub + restart the
  // platform keep-alive if it was left enabled).
  onMount(() => initLiveSync());
  // Read the MCP server's state (the backend restarts it on launch if it was
  // left on), so the Settings toggle shows the truth.
  onMount(() => void initMcp());
  // Read configured linked folders (#219) so Settings shows them; the backend
  // resumes any enabled ones on its own.
  onMount(() => void initLinkFolders());
  // Keep the Settings toggle in step when background sync is changed from the
  // desktop tray ("Turn off background sync").
  onMount(() => {
    let un: (() => void) | undefined;
    onBackgroundSyncChanged(applyLiveSyncFromBackend).then((u) => (un = u));
    return () => un?.();
  });
  onMount(() => {
    if (!isMacApp) return;
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

  // ---- Mobile swipe gestures (#46) ----
  // A horizontal swipe in the middle of the screen opens (right, when closed) or
  // dismisses (left, when open) the drawer. Window-level touch listeners in the
  // capture phase (see onMount) so we see the touch before CodeMirror/scrollers
  // grab it; once a horizontal drag is locked we preventDefault to stop their
  // scroll/selection. `drawerPan` is the live translateX (px), null when idle.
  const DRAWER_W = 256;
  const EDGE = 24;
  const SLOP = 8;
  let panStart: { x: number; y: number; opening: boolean } | null = null;
  let panLocked = false;
  let drawerPan = $state<number | null>(null);
  // Where the current touch began, and whether it has travelled far enough to
  // be a drag rather than a tap — the same SLOP the drawer uses to tell those
  // apart. Nobody taps perfectly still, so a tap fires touchmove too, and
  // counting that as scrolling would let the tap arm the gate for the editor's
  // own scroll a few milliseconds later (#248).
  let touchOrigin: { x: number; y: number } | null = null;
  let touchDragging = false;

  function onSwipeStart(e: TouchEvent) {
    // The finger that just landed, which is not `touches[0]` when another one is
    // already down — taking that one would move the origin to wherever the
    // existing finger has got to, and forget that it was already dragging.
    const landed = e.changedTouches[0];
    if (landed && e.touches.length === 1) {
      touchOrigin = { x: landed.clientX, y: landed.clientY };
      touchDragging = false;
    }
    if (!mobile || settingsOpen || e.touches.length !== 1) return;
    const t = e.touches[0];
    if (t.clientX <= EDGE || t.clientX >= window.innerWidth - EDGE) return;
    panStart = { x: t.clientX, y: t.clientY, opening: !sidebarOpen };
    panLocked = false;
  }
  function onSwipeMove(e: TouchEvent) {
    // Ahead of the drawer-swipe checks: this is the record of the user
    // scrolling, and those checks bail out on exactly the vertical drags that
    // do it (#248). Latched once past SLOP, so a drag that wanders back toward
    // where it started keeps counting.
    const moved = e.touches[0];
    if (moved && touchOrigin) {
      if (!touchDragging) {
        const dist = Math.hypot(moved.clientX - touchOrigin.x, moved.clientY - touchOrigin.y);
        if (dist > SLOP) touchDragging = true;
      }
      if (touchDragging) lastScrollGestureAt = Date.now();
    }
    if (!panStart || e.touches.length !== 1) return;
    const t = e.touches[0];
    const dx = t.clientX - panStart.x;
    const dy = t.clientY - panStart.y;
    if (!panLocked) {
      if (Math.abs(dx) < SLOP && Math.abs(dy) < SLOP) return;
      if (Math.abs(dy) >= Math.abs(dx)) {
        panStart = null;
        return;
      }
      panLocked = true;
    }
    e.preventDefault();
    const base = panStart.opening ? -DRAWER_W : 0;
    drawerPan = Math.max(-DRAWER_W, Math.min(0, base + dx));
  }
  const COMMIT = 64;
  function onWheelGesture() {
    lastScrollGestureAt = Date.now();
  }
  function onSwipeEnd(e: TouchEvent) {
    // Only once the last finger has gone: lifting one of two leaves a drag in
    // progress, and forgetting its origin would stop the rest of it counting as
    // scrolling. Bookkeeping for the next touch either way — the fling after
    // this one is already covered by the timestamp, and a touchend we never see
    // (the quick-edit path unmounts the tapped node) is corrected by the next
    // touchstart rather than leaving anything armed.
    if (e.touches.length === 0) {
      touchOrigin = null;
      touchDragging = false;
    }
    if (panStart && panLocked && drawerPan !== null)
      setSidebar(panStart.opening ? drawerPan >= COMMIT - DRAWER_W : drawerPan > -COMMIT);
    panStart = null;
    panLocked = false;
    drawerPan = null;
  }

  // Search palette (#15). `searchInitial` seeds the query when opened from a
  // tag chip in the preview, so the tag is pre-filtered.
  let searchOpen = $state(false);
  let searchInitial = $state("");
  // The open vault and the active tab's note both live in the session store, so
  // the tab strip, the sidebar highlight and the editor can't disagree about
  // which note is open (#169).
  const activeVault = $derived(session.vault);
  const activePath = $derived(session.path);
  // Whether the soft keyboard is up. The toolbar is anchored to the keyboard, so
  // it must hide when the keyboard is dismissed even if the editor keeps focus.
  let kbOpen = $state(false);

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
  // All note paths in the open vault, for resolving [[internal links]].
  const notePaths = $derived([...walk(tree)].filter((n) => !n.is_dir).map((n) => n.path));

  // Set true when opening a brand-new note: force source mode and focus the
  // editor once it mounts (#50). The pane clears it once it has.
  let focusNewNote = $state(false);
  // The active tab's panes (#169) — one, or two side by side. Each owns the
  // buffer, the saving and the scroll for the note it shows; App owns which
  // notes those are. `flush` parks them before the notes change, which is the
  // one ordering rule between the two.
  let paneRefs = $state<(NotePane | undefined)[]>([]);
  // Seeded for both panes: `bind:` to an unset slot is refused when the prop
  // has a fallback, and two panes is the ceiling.
  let paneNoteTypes = $state<NoteType[]>(["markdown", "markdown"]);
  // The tab on screen, and the pane within it that has the keyboard: what the
  // header's controls, the hotkeys and the settings sheet all act on.
  const tab = $derived(session.tab);
  const focusedPane = $derived(tab?.focused ?? 0);
  // Which of the tab's notes are on screen, with the index each one has in the
  // tab. A split needs room, so a phone shows the focused side only — the tab
  // stays joined, and widening the window brings the other back. (Opening a
  // note on mobile collapses the strip to one tab, but a session restored from
  // a desktop can still arrive holding a split.)
  const shownPanes = $derived.by(() => {
    const t = tab;
    if (!t) return [];
    if (mobile) return [{ p: t.panes[t.focused], i: t.focused }];
    return t.panes.map((p, i) => ({ p, i }));
  });
  const pane = $derived(paneRefs[focusedPane]);
  const singleView = $derived(
    !!activePath && noteTypeInfo(paneNoteTypes[focusedPane] ?? "markdown").singleView,
  );
  /// Park every open note. False when a save was refused, in which case the
  /// caller leaves the notes where they are (#253).
  async function flushSave(): Promise<boolean> {
    for (const p of paneRefs) if (p && !(await p.flush())) return false;
    return true;
  }

  // A `[[note#heading]]` link: open the note (unless already open) and scroll
  // the preview to the heading. Headings get their slug ids after Preview
  // renders, so retry across a few frames until the anchor exists (#45).
  // Open the search palette with an empty query. Shared by Cmd/Ctrl+F and the
  // mobile search button (#209), so both clear any tag seeded by a previous
  // open rather than one path forgetting to.
  function openSearch() {
    searchInitial = "";
    searchOpen = true;
  }

  // Clicking a #tag — in preview or in source mode (#202) — opens search on it.
  // The query is exactly "#tag": the backend reads that as a tag query and
  // matches whole tags, so no trailing space may be added here.
  function openTagSearch(tag: string) {
    searchInitial = `#${tag}`;
    searchOpen = true;
  }

  function openInternalLink(path: string, fragment: string | undefined) {
    if (!activeVault) return;
    if (path !== activePath) openNote(activeVault, path);
    if (fragment) pane?.scrollToAnchor(fragment);
  }

  // ---- Tabs (#169) ----
  //
  // The tab list is the session store's, and the note it points at is handed to
  // the pane as props — so switching tabs is a session write, and the pane
  // reloads itself. What App must do first is park the outgoing note: flushing
  // *before* the session changes is the last moment a pending edit and scroll
  // position can be written to the note they belong to.

  async function openNote(
    vault: string,
    path: string,
    opts: { focus?: boolean; pin?: boolean; newTab?: boolean } = {},
  ) {
    // A path that has left the tabs was just renamed or deleted (those callers
    // flush before they touch the vault), so there is nothing left to send.
    const open = session.path;
    const stale =
      !!open && !session.tabs.some((t) => t.panes.some((pane) => pane.path === open));
    if (!stale && path !== open && !(await flushSave())) return;
    // Opening a note is what dismisses the drawer, whether or not it was
    // already the open one.
    if (mobile) setSidebar(false);
    session.vault = vault; // closes the tabs if this is a different vault
    // Mobile keeps the single-note view — there is no strip to switch or close
    // tabs with, so opening a note there closes the last one rather than
    // stacking up a list nobody can see.
    if (mobile && path !== session.path) session.closeAll();
    // A note we just created is pinned: you're about to type in it, and a
    // preview tab would be replaced by the next thing clicked in the sidebar.
    session.open(path, { pin: opts.pin || opts.focus, newTab: opts.newTab });
    // A new note always opens in source mode so the user can type right away.
    if (opts.focus) session.mode = "source";
    focusNewNote = !!opts.focus;
  }

  async function selectTab(i: number) {
    if (i === session.active) return;
    if (!(await flushSave())) return;
    session.activate(i);
  }

  /**
   * Double-clicking a tab promotes it: a preview tab gets pinned, and a tab
   * that is already pinned opens the rename dialog — which is where the
   * breadcrumb's double-click-to-rename went when tabs took its place (#52).
   */
  async function tabDblClick(i: number) {
    const t = session.tabs[i];
    if (!t) return;
    if (t.preview) {
      session.pin(i);
      return;
    }
    // A joined tab names two notes, so there is no "the note" to rename from
    // it — the tree's context menu and the settings sheet still do.
    if (t.panes.length > 1) return;
    const path = t.panes[0].path;
    await selectTab(i);
    if (session.active === i && session.tabs[i]?.panes[0]?.path === path)
      sidebar?.renameActive();
  }

  /// A tab dropped onto another: the two notes become one tab showing both.
  async function joinTabs(from: number, to: number) {
    if (!(await flushSave())) return;
    session.join(from, to);
  }

  /// The split-apart button: two notes, two tabs again.
  async function splitTab(i: number) {
    if (!(await flushSave())) return;
    session.split(i);
  }

  // The row the panes share, for turning a divider drag into a ratio.
  let paneRow = $state<HTMLElement | undefined>(undefined);
  function dragDivider(clientX: number) {
    const box = paneRow?.getBoundingClientRect();
    if (!box || box.width === 0 || session.active === -1) return;
    session.ratio(session.active, (clientX - box.left) / box.width);
  }
  function nudgeDivider(dir: -1 | 1) {
    const t = session.tab;
    if (!t || session.active === -1) return;
    session.ratio(session.active, t.ratio + dir * 0.02);
  }

  async function closeTab(i: number) {
    // Only the active tab holds a buffer, so only closing that one can lose an
    // edit that hasn't landed yet.
    if (i === session.active && !(await flushSave())) return;
    session.close(i);
  }

  // A note (or a folder of them) renamed anywhere — the sidebar's rename, a
  // drag in the tree, or Move in the settings sheet. Tabs follow it, including
  // background ones, so none is left pointing at a path that no longer exists.
  async function notesRenamed(from: string, to: string, isDir: boolean) {
    for (const p of paneRefs) p?.renamed(from, to, isDir);
    session.renamed(from, to, isDir);
  }

  // Deleted, likewise: its tabs go. Nothing is flushed — the note is gone.
  async function notesRemoved(path: string, isDir: boolean) {
    session.removed(path, isDir);
  }

  async function handleVaultChange(vault: string | null) {
    // Only worth flushing when we're leaving a vault we had open; the launch
    // sequence reports the restored vault as a "change" with nothing loaded.
    if (session.path) await flushSave();
    // Setting the vault closes the tabs when it differs from the one they were
    // opened in — and keeps them when it doesn't, which is the launch restore.
    session.vault = vault;
    needsPrune = true;
  }

  // Restored tabs are pruned against the first tree we see for the vault: a
  // note may have been deleted on another device since we last ran. Once only —
  // a note created later is opened before it reaches the tree, and pruning
  // again would close it.
  let needsPrune = true;
  async function handleTree(t: TreeNode[]) {
    tree = t;
    if (!needsPrune || !session.vault) return;
    needsPrune = false;
    const files = new Set([...walk(t)].filter((n) => !n.is_dir).map((n) => n.path));
    session.prune((p) => files.has(p));
  }

  // FAB / Cmd+N: one tap creates and opens an "Untitled" note in the current
  // note's folder (root if none) — no name prompt — so the user can type right
  // away (#85). Creation lives in the sidebar alongside its other dialogs.
  function newNoteHere() {
    sidebar?.newUntitledNote(currentDir);
  }

  // Breadcrumb rename (mobile, where the header carries the note's path):
  // long-press opens the sidebar's rename prompt for the active note (#52).
  // Desktop reaches the same prompt by double-clicking the note's tab.
  let crumbPressTimer: ReturnType<typeof setTimeout> | undefined;
  function crumbPressStart() {
    crumbPressTimer = setTimeout(() => {
      crumbPressTimer = undefined;
      sidebar?.renameActive();
    }, 500);
  }
  function crumbPressEnd() {
    clearTimeout(crumbPressTimer);
  }

  // File actions (from the settings sheet).
  async function moveNote(dir: string) {
    if (!activeVault || !activePath) return;
    // Land any pending edit before the note moves out from under it.
    await flushSave();
    const from = activePath;
    const base = from.split("/").pop()!;
    const to = dir ? `${dir}/${base}` : base;
    await renamePath(activeVault, from, to, false);
    await notesRenamed(from, to, false);
  }
  // Duplicate the note in the same folder as "X (copy).md" (or "X (copy N).md").
  async function duplicateNote() {
    if (!activeVault || !activePath) return;
    const finalPath = await duplicateNoteFile(activeVault, activePath, tree);
    openNote(activeVault, finalPath, { pin: true });
  }
  // Import/export run through native dialogs + the backend; surface any failure
  // instead of letting the promise reject silently (#79).
  function reportTransferError(e: unknown) {
    console.error("import/export failed", e);
  }
  async function onImportNote() {
    if (!activeVault) return;
    try {
      const created = await importNoteMd(activeVault, currentDir);
      if (created) openNote(activeVault, created, { pin: true });
    } catch (e) {
      reportTransferError(e);
    }
  }
  async function deleteNote() {
    if (!activeVault || !activePath) return;
    const path = activePath;
    await deletePath(activeVault, path, false);
    await notesRemoved(path, false);
  }

  onMount(() => {
    // Reactive mobile flag — gates the FAB + markdown toolbar on resize/rotate.
    const mq = window.matchMedia("(max-width: 767px)");
    const onMq = (e: MediaQueryListEvent) => (mobile = e.matches);
    mq.addEventListener("change", onMq);
    window.addEventListener("keydown", onKeydown);
    window.addEventListener("touchstart", onSwipeStart, { capture: true, passive: true });
    window.addEventListener("touchmove", onSwipeMove, { capture: true, passive: false });
    window.addEventListener("touchend", onSwipeEnd, { capture: true });
    window.addEventListener("touchcancel", onSwipeEnd, { capture: true });
    // A wheel counts as the user scrolling too: `mobile` is a width query, so a
    // narrow window or a tablet with a mouse gets this layout with no touches
    // at all, and the chrome would never hide for them (#248).
    window.addEventListener("wheel", onWheelGesture, { capture: true, passive: true });

    // Detect the soft keyboard from the visual viewport — see $lib/soft-keyboard
    // for the rule, which has to account for rotation as well as the keyboard.
    const vv = window.visualViewport;
    const keyboard = softKeyboard();
    const onVv = () => {
      if (!vv) return;
      kbOpen = keyboard.sample(
        window.innerWidth,
        vv.height,
        document.documentElement.clientHeight,
      );
    };
    if (vv) {
      onVv();
      vv.addEventListener("resize", onVv);
      vv.addEventListener("scroll", onVv);
    }
    // Rotation changes the layout viewport without necessarily touching the
    // visual one, and the baseline is per layout — so it needs re-sampling.
    window.addEventListener("resize", onVv);
    window.addEventListener("orientationchange", onVv);

    return () => {
      mq.removeEventListener("change", onMq);
      window.removeEventListener("keydown", onKeydown);
      window.removeEventListener("touchstart", onSwipeStart, { capture: true });
      window.removeEventListener("touchmove", onSwipeMove, { capture: true });
      window.removeEventListener("touchend", onSwipeEnd, { capture: true });
      window.removeEventListener("touchcancel", onSwipeEnd, { capture: true });
      window.removeEventListener("wheel", onWheelGesture, { capture: true });
      vv?.removeEventListener("resize", onVv);
      vv?.removeEventListener("scroll", onVv);
      window.removeEventListener("resize", onVv);
      window.removeEventListener("orientationchange", onVv);
    };
  });

  // Global hotkeys. Mod = Cmd (macOS) / Ctrl (elsewhere). These complement the
  // editor's own text-formatting shortcuts, which CodeMirror handles internally.
  // Keys that scroll. A hardware keyboard produces neither touches nor wheel
  // events, so without this the chrome never moves for someone reading with the
  // keyboard at a narrow width — `mobile` is a width query, not a pointer one
  // (#250).
  const SCROLL_KEYS = new Set([
    "ArrowUp",
    "ArrowDown",
    "PageUp",
    "PageDown",
    "Home",
    "End",
    " ",
  ]);

  function onKeydown(e: KeyboardEvent) {
    if (SCROLL_KEYS.has(e.key)) lastScrollGestureAt = Date.now();
    // ESC leaves a desktop quick edit (#153), returning to preview. Gated on
    // quickEditActive so it only undoes a double-click-to-edit, not a manual
    // source view (where ESC belongs to CodeMirror — closing autocomplete, etc.).
    if (e.key === "Escape") {
      // Only when it had a desktop quick edit to leave; otherwise ESC belongs to
      // CodeMirror (closing autocomplete, etc.).
      if (pane?.escapeQuickEdit()) e.preventDefault();
      return;
    }
    const mod = e.metaKey || e.ctrlKey;
    if (!mod) return;
    const key = e.key.toLowerCase();
    if (key === "\\") {
      // Toggle the sidebar.
      e.preventDefault();
      setSidebar(!sidebarOpen);
    } else if (key === ",") {
      // Toggle settings open/closed.
      e.preventDefault();
      settingsOpen = !settingsOpen;
    } else if (key === "n" && e.shiftKey) {
      // New folder (at vault root).
      e.preventDefault();
      sidebar?.createFolderHotkey();
    } else if (key === "n") {
      // New note (in the current note's folder).
      e.preventDefault();
      newNoteHere();
    } else if (key === "f") {
      // Search this vault (#15). Cmd+F is the browser's find-in-page, which is
      // useless here — the note is a CodeMirror document, not page text — so
      // taking it is an upgrade rather than a loss.
      e.preventDefault();
      openSearch();
    } else if (key === "p") {
      // Toggle source <-> preview (only meaningful with a note open, and only
      // for Markdown notes — typed notes have a single view).
      if (activePath && !singleView) {
        e.preventDefault();
        pane?.toggleMode();
      }
    } else if (key === "w") {
      // Close the active tab. Cmd+W reaches us because the macOS menu's Close
      // Window moved to Cmd+Shift+W (see setup_macos_menu) — a native
      // accelerator would otherwise swallow the key before the webview.
      if (!mobile && session.active !== -1) {
        e.preventDefault();
        closeTab(session.active);
      }
    } else if (!mobile && key >= "1" && key <= "9") {
      // Jump straight to a tab by position (#169).
      const i = Number(key) - 1;
      if (i < session.tabs.length) {
        e.preventDefault();
        selectTab(i);
      }
    }
  }
</script>

<div
  class="relative flex h-screen w-screen flex-col overflow-hidden bg-background text-foreground"
  style="padding-left:env(safe-area-inset-left);padding-right:env(safe-area-inset-right);padding-bottom:env(safe-area-inset-bottom);--chrome-h:{mobile
    ? headerH + 'px'
    : '0px'};"
>
  <!-- Top bar. data-tauri-drag-region lets the window drag by the header, since the
       Overlay titlebar removes the native drag strip; child buttons still click. -->
  <!-- On mobile the header floats (absolute) fully over the scroller, which
       carries a *constant* top padding of --chrome-h (the header height) so
       content always clears it. Hiding is a pure transform + fade that never
       touches layout, so it can't nudge scrollTop — the padding just scrolls off
       the top like any content, reclaiming the space, and scrolls back into view
       (with the header flying back in) on scroll-up (#100). Desktop keeps the
       header in normal flow. -->
  <header
    data-tauri-drag-region
    bind:offsetHeight={headerH}
    class="flex shrink-0 items-center justify-between gap-3 border-b border-border {isMacApp
      ? 'min-h-0 px-2'
      : 'min-h-12 px-3 pb-2'} {mobile
      ? 'absolute inset-x-0 top-0 z-10 bg-background'
      : 'bg-secondary/40'}"
    style="padding-top:calc(env(safe-area-inset-top) + {isMacApp
      ? '0.25rem'
      : '0.5rem'});{isMacApp ? 'padding-bottom:0.25rem;' : ''}{isMacApp && !fullscreen
      ? 'padding-left:78px;'
      : ''}{mobile
      ? `transition:transform 200ms ease, opacity 200ms ease;transform:translateY(${chromeHidden
          ? '-100%'
          : '0'});opacity:${chromeHidden ? 0 : 1};`
      : ''}"
  >
    <div data-tauri-drag-region class="flex min-w-0 flex-1 items-center gap-2">
      <button
        type="button"
        class="shrink-0 rounded text-muted-foreground hover:bg-muted hover:text-foreground {isMacApp
          ? 'p-1'
          : 'p-1.5'}"
        aria-label="Toggle sidebar"
        aria-pressed={sidebarOpen}
        title="Toggle sidebar"
        onclick={() => setSidebar(!sidebarOpen)}
      >
        <PanelLeft size={chromeIcon} />
      </button>
      {#if mobile}
        <!-- Tail path rendering: when the path is too long, the ellipsis collapses
             the *leading* path (left) so the filename stays visible. The container
             is rtl (so overflow/ellipsis lands on the left); an inner `dir="ltr"`
             override keeps the path itself reading left-to-right. -->
        <!-- Base color is the muted/60 of the leading segments so the truncation
             ellipsis matches them; the active segment overrides with text-foreground. -->
        <span
          data-tauri-drag-region
          class="path-crumb text-sm font-medium text-muted-foreground/60"
        >
          {#if activePath}
            {@const parts = activePath.replace(/\.md$/, "").split("/")}
            <bdo dir="ltr">
              {#each parts as seg, i}
                {#if i > 0}<span class="mx-1.5 text-muted-foreground/40">/</span
                  >{/if}{#if i === parts.length - 1}<!-- The filename: long-press
                  to rename the open note (#52). -->
                  <!-- svelte-ignore a11y_no_static_element_interactions -->
                  <span
                    class="cursor-text text-foreground"
                    title="Long-press to rename"
                    onpointerdown={crumbPressStart}
                    onpointerup={crumbPressEnd}
                    onpointerleave={crumbPressEnd}
                    oncontextmenu={(e) => e.preventDefault()}>{seg}</span
                  >{:else}<span class="text-muted-foreground/60">{seg}</span>{/if}
              {/each}
            </bdo>
          {/if}
        </span>
      {:else}
        <!-- Desktop: the open notes as tabs, where the breadcrumb sits on mobile
             (#169). A tab shows the filename with the full path as its tooltip,
             and the strip keeps the header's drag region (see TabStrip). -->
        <TabStrip
          tabs={session.tabs}
          active={session.active}
          onselect={selectTab}
          onclose={closeTab}
          ondblclick={tabDblClick}
          onreorder={(from, to) => session.move(from, to)}
          onjoin={joinTabs}
          onsplit={splitTab}
        />
      {/if}
    </div>

    <div data-tauri-drag-region class="flex items-center gap-1.5">
      <!-- Single toggle: click anywhere flips Source<->Preview; active half is lit.
           Hidden for typed notes (#104), which have exactly one view. -->
      {#if !singleView}
      <button
        type="button"
        class="inline-flex items-center gap-0.5 rounded-full border border-border bg-background p-0.5"
        aria-label="Toggle view mode"
        title={mode === "source" ? "Switch to preview" : "Switch to source"}
        onclick={() => pane?.toggleMode()}
      >
        <span
          class="flex items-center justify-center rounded-full transition-colors {isMacApp
            ? 'p-1'
            : 'p-1.5'} {mode === 'source'
            ? 'bg-primary text-primary-foreground'
            : 'text-muted-foreground'}"
        >
          <Code size={chromeIcon} />
        </span>
        <span
          class="flex items-center justify-center rounded-full transition-colors {isMacApp
            ? 'p-1'
            : 'p-1.5'} {mode === 'preview'
            ? 'bg-primary text-primary-foreground'
            : 'text-muted-foreground'}"
        >
          <Eye size={chromeIcon} />
        </span>
      </button>
      {/if}
      <!-- Search (#209). Desktop has Cmd/Ctrl+F; mobile has no hardware keyboard,
           so without this button the palette — and therefore search and tags —
           is unreachable there. Shown only where the hotkey isn't available, so
           the desktop chrome doesn't gain a redundant control. -->
      {#if mobile && activeVault}
        <button
          type="button"
          class="flex items-center justify-center rounded-full p-2 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
          aria-label="Search notes"
          title="Search notes"
          onclick={openSearch}
        >
          <Search size={chromeIcon} />
        </button>
      {/if}
      <button
        type="button"
        class="flex items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-muted hover:text-foreground {isMacApp
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

  <!-- Status-bar scrim (mobile). We leave the Android system status bar alone, so
       when the chrome is hidden the note scrolls up underneath it; this fixed
       fade softens text against the status bar. It's a solid background masked to
       fade its alpha (so the same theme colour fades cleanly to nothing, with no
       grey mid-tone) — a top→transparent gradient over the status-bar inset. Sits
       under the header (z-10) so the header covers it while shown. -->
  {#if mobile}
    <div
      aria-hidden="true"
      class="pointer-events-none absolute inset-x-0 top-0 z-[5]"
      style="height:calc(env(safe-area-inset-top) + 16px);background:var(--background);-webkit-mask-image:linear-gradient(to bottom, #000, transparent);mask-image:linear-gradient(to bottom, #000, transparent);"
    ></div>
  {/if}

  <!-- Body -->
  <div class="relative flex min-h-0 flex-1">
    <!-- Mobile backdrop. Shown while open or mid-drag; its opacity tracks the
         drawer position during an interactive swipe (#46). -->
    {#if sidebarOpen || drawerPan !== null}
      <button
        type="button"
        class="fixed inset-0 z-20 bg-black/50 md:hidden"
        style={drawerPan !== null
          ? `opacity:${(drawerPan + DRAWER_W) / DRAWER_W}`
          : ""}
        aria-label="Close sidebar"
        onclick={() => setSidebar(false)}
      ></button>
    {/if}

    <!-- Collapsible on desktop, drawer on mobile. While dragging (drawerPan set)
         we drive translateX inline with no transition so it tracks the finger;
         on release drawerPan clears and the class transition snaps it home. -->
    <aside
      class="z-30 shrink-0 overflow-hidden border-border bg-secondary transition-all duration-200 ease-out fixed inset-y-0 left-0 w-64 md:static md:z-auto md:bg-secondary/40 {sidebarOpen
        ? 'translate-x-0 border-r md:w-64'
        : '-translate-x-full border-r-0 md:translate-x-0 md:w-0'}"
      style={drawerPan !== null
        ? `transform:translateX(${drawerPan}px);transition:none`
        : ""}
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
          onopen={openNote}
          onvaultchange={handleVaultChange}
          ontree={handleTree}
          onrenamed={notesRenamed}
          onremoved={notesRemoved}
        />
      </div>
    </aside>

    <!-- The open tab's notes: one pane, or two side by side with a draggable
         divider (#169). Each pane owns its note's buffer, saving and scroll —
         App only says which notes, and parks them (flush) before that changes.
         A grid rather than flex so the ratio is the column definition, and
         neither pane's content can push the split around. -->
    {#if activeVault && tab}
      <div
        bind:this={paneRow}
        class="grid min-w-0 flex-1"
        style={shownPanes.length > 1
          ? `grid-template-columns:${tab.ratio}fr auto ${1 - tab.ratio}fr;`
          : "grid-template-columns:1fr;"}
      >
        {#each shownPanes as { p, i } (p.path)}
          {#if i > 0}
            <Splitter
              label="Resize panes"
              value={tab.ratio * 100}
              ondrag={dragDivider}
              onnudge={nudgeDivider}
              onreset={() => session.ratio(session.active, 0.5)}
            />
          {/if}
          <div class="flex min-w-0 overflow-hidden">
            <NotePane
              bind:this={paneRefs[i]}
              bind:noteType={paneNoteTypes[i]}
              bind:focusNew={focusNewNote}
              vault={activeVault}
              path={p.path}
              mode={p.mode}
              scroll={p.scroll}
              {mobile}
              {kbOpen}
              {headerH}
              {notePaths}
              {userScrolling}
              onmode={(m) => session.setPane(session.active, i, { mode: m })}
              onscrollratio={(r) => session.setPane(session.active, i, { scroll: r })}
              onchrome={(hidden) => (chromeHidden = hidden)}
              onedit={() => session.pinActive()}
              onactivate={() => session.focus(session.active, i)}
              onopen={(path, opts) => activeVault && openNote(activeVault, path, opts)}
              ontag={openTagSearch}
              oninternallink={openInternalLink}
            />
          </div>
        {/each}
      </div>
    {:else}
      <div class="flex min-w-0 flex-1 flex-col items-center justify-center gap-3 text-muted-foreground">
        <NotebookPen size={40} class="opacity-30" />
        <p class="text-sm">Select or create a note.</p>
      </div>
    {/if}
  </div>
</div>

<!-- Mobile: floating new-note button. Always available while a vault is open
     (#85); auto-hides on scroll-down, slides back in on scroll-up. -->
{#if mobile && activeVault}
  <Fab
    onclick={newNoteHere}
    ontype={(type) => sidebar?.newTypedNote(currentDir, type)}
    hidden={chromeHidden}
  />
{/if}

<SearchPalette
  bind:open={searchOpen}
  vault={activeVault}
  initial={searchInitial}
  onopen={(path) => activeVault && openNote(activeVault, path)}
/>

<SettingsSheet
  bind:open={settingsOpen}
  {activePath}
  {folders}
  {currentDir}
  onmove={moveNote}
  onduplicate={duplicateNote}
  oncopy={() => pane?.copyContents()}
  ondelete={deleteNote}
  onrename={() => sidebar?.renameActive()}
  onexportnote={() =>
    activeVault && activePath && exportNoteMd(activeVault, activePath).catch(reportTransferError)}
  onsharenote={() =>
    activeVault && activePath && shareNote(activeVault, activePath).catch(reportTransferError)}
  onemailnote={() =>
    activeVault && activePath && emailNote(activeVault, activePath).catch(reportTransferError)}
  onexportvault={() => activeVault && exportVaultZip(activeVault, "").catch(reportTransferError)}
  onimportvault={() => activeVault && importVaultZip(activeVault).catch(reportTransferError)}
  onimportnote={onImportNote}
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
