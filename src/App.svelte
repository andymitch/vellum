<script lang="ts">
  import { onMount, tick } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import type { EditorView } from "@codemirror/view";
  import Editor from "$lib/components/editor/Editor.svelte";
  import Preview from "$lib/components/editor/Preview.svelte";
  import TodoList from "$lib/components/editor/TodoList.svelte";
  import JournalView from "$lib/components/editor/JournalView.svelte";
  import Sidebar from "$lib/components/sidebar/Sidebar.svelte";
  import MarkdownToolbar from "$lib/components/editor/MarkdownToolbar.svelte";
  import SettingsSheet from "$lib/components/SettingsSheet.svelte";
  import SearchPalette from "$lib/components/SearchPalette.svelte";
  import { checkForUpdate, checkForUpdateMobile } from "$lib/updater";
  import Fab from "$lib/components/Fab.svelte";
  import {
    readNote,
    writeNote,
    renamePath,
    deletePath,
    shareNote,
    onVaultChanged,
    onBackgroundSyncChanged,
    type TreeNode,
  } from "$lib/vault";
  import { session } from "$lib/session.svelte";
  import { slugify } from "$lib/slug";
  import { duplicateNote as duplicateNoteFile } from "$lib/notes";
  import {
    exportVaultZip,
    importVaultZip,
    exportNoteMd,
    importNoteMd,
    emailNote,
  } from "$lib/transfer";
  import { editorSettings } from "$lib/editor-settings.svelte";
  import { initLiveSync, applyLiveSyncFromBackend } from "$lib/live-sync.svelte";
  import { initMcp } from "$lib/mcp.svelte";
  import { initLinkFolders } from "$lib/link-folders.svelte";
  import { noteTypeInfo, noteTypeOf, countChecked, sweepChecked } from "$lib/note-type";
  import {
    Code,
    Eye,
    PanelLeft,
    NotebookPen,
    Settings,
    BrushCleaning,
    Search,
  } from "@lucide/svelte";

  type Mode = "source" | "preview";
  let mode = $state<Mode>(session.mode);
  // The scrollable element differs by mode: CodeMirror scrolls inside its own
  // `.cm-scroller`, while the preview scrolls the <main> element itself.
  let mainEl = $state<HTMLElement | undefined>(undefined);
  function scrollerFor(m: Mode): HTMLElement | null {
    if (!mainEl) return null;
    return m === "source" ? mainEl.querySelector<HTMLElement>(".cm-scroller") : mainEl;
  }
  // Current scroll as a 0..1 ratio of the active view (so it maps across the
  // differently-sized source/preview views).
  function scrollRatio(m: Mode): number {
    const el = scrollerFor(m);
    const max = el ? el.scrollHeight - el.clientHeight : 0;
    return el && max > 0 ? el.scrollTop / max : 0;
  }
  function setScroll(m: Mode, ratio: number) {
    const el = scrollerFor(m);
    if (!el) return;
    const max = el.scrollHeight - el.clientHeight;
    if (max > 0) {
      el.scrollTop = ratio * max;
      // Keep the auto-hide baseline in sync: a programmatic jump (mode toggle /
      // launch restore) must not read as the user scrolling down and hide the
      // chrome (#85).
      lastChromeTop = el.scrollTop;
    }
  }
  // Apply a 0..1 ratio to a mode's scroller after it has laid out. Two rAFs:
  // a freshly-mounted CodeMirror needs a frame to measure a tall document.
  function applyScroll(m: Mode, ratio: number) {
    requestAnimationFrame(() => requestAnimationFrame(() => setScroll(m, ratio)));
  }
  // Restore variant for cold boot: a freshly-created editor refines a tall
  // document's height over several frames, so re-apply across a short window
  // until the layout settles. Used only on launch (not on every toggle, where a
  // late jump would fight the user).
  function applyScrollRestore(m: Mode, ratio: number) {
    if (ratio <= 0) return;
    for (const d of [0, 80, 200, 400]) setTimeout(() => setScroll(m, ratio), d);
  }
  // Toggle source <-> preview, preserving the scroll position. The two views
  // have different (and recreated) scrollers, so carry the scroll *ratio* across.
  async function setMode(m: Mode) {
    if (m === mode) return;
    // Leaving source ends any quick-edit session, so ESC (desktop) / keyboard
    // dismiss (mobile) only returns to preview for a source we entered that way.
    if (m === "preview") quickEditActive = false;
    const ratio = scrollRatio(mode);
    resetChrome();
    mode = m;
    session.mode = m;
    session.scroll = ratio;
    await tick();
    // A quick-edit tap positions source's scroll itself (caret pinned to the tap's
    // on-screen height, see the focus effect); ratio-restore would fight that jump.
    if (!(m === "source" && quickEditActive)) applyScroll(m, ratio);
  }

  // Auto-hide the editor chrome (top bar + FAB) on scroll-down, reveal on
  // scroll-up, so the reading/writing surface is unobstructed on small screens
  // while the controls stay one gesture away (#85). Driven off the same scroll
  // events as the save below — direction is computed synchronously (not
  // debounced) so the chrome responds immediately.
  let chromeHidden = $state(false);
  // Seed near the real height (min-h-12 + pb-2) so the mobile body's
  // padding-top:headerH doesn't jump on the first frame before offsetHeight
  // binds (#100). The bind corrects it (incl. safe-area) a frame later.
  let headerH = $state(56);
  let lastChromeTop = 0;
  let lastScroller: HTMLElement | null = null;
  function resetChrome() {
    chromeHidden = false;
    lastChromeTop = 0;
  }
  // The chrome floats over the scroller, which carries a *constant* top padding
  // equal to the header height (see markup). Hiding the chrome is a pure
  // transform+fade that never changes layout, so it can't nudge scrollTop — no
  // settling window is needed. The padding just scrolls off the top like any
  // other content: scrolling down reclaims its space, scrolling back up brings
  // it (and the chrome) back (#100).
  //
  // Persist the open note's scroll (debounced) so launch can restore it. A
  // capturing listener catches scroll from either scroller (scroll doesn't
  // bubble, but it is observable in the capture phase).
  let scrollSaveTimer: ReturnType<typeof setTimeout> | undefined;
  function onAnyScroll() {
    // Show/hide chrome by scroll direction (small threshold to ignore jitter).
    const el = scrollerFor(mode);
    if (el) {
      const top = el.scrollTop;
      const scrollable = el.scrollHeight - el.clientHeight;
      if (el !== lastScroller) {
        // Scroller swapped (mode toggle / note remount): re-baseline so the
        // position jump isn't read as a user scroll and hide the chrome.
        lastScroller = el;
        lastChromeTop = top;
      } else if (scrollable < 120) {
        // Too little scroll room to bother hiding the chrome — keep it shown.
        // Hiding to reveal a sliver just flickers the bars on a note that barely
        // overflows (#85).
        chromeHidden = false;
        lastChromeTop = top;
      } else {
        const delta = top - lastChromeTop;
        if (top < 8) chromeHidden = false;
        else if (delta > 6) chromeHidden = true;
        else if (delta < -6) chromeHidden = false;
        lastChromeTop = top;
      }
    }
    clearTimeout(scrollSaveTimer);
    scrollSaveTimer = setTimeout(() => {
      if (activePath) session.scroll = scrollRatio(mode);
    }, 150);
  }
  // Captured at init, before the sidebar's launch sequence clears session.path
  // (via onvaultchange). Used to restore scroll for the note reopened on launch.
  const initialPath = session.path;
  const initialScroll = session.scroll;
  // Set once we've handled the first (launch-restore) note open, so subsequent
  // opens start at the top instead of inheriting the restored ratio.
  let didFirstOpen = false;

  const mobileInit = window.matchMedia("(max-width: 767px)").matches;
  let mobile = $state(mobileInit);
  // macOS desktop uses an Overlay titlebar (traffic lights float over our header,
  // so the window chrome takes the header's color). The header is compacted to the
  // titlebar height and its left edge is inset to clear the lights — except in
  // fullscreen, where macOS hides them and the toggle can sit flush left.
  const isMacDesktop = /Macintosh/.test(navigator.userAgent) && !/Android/.test(navigator.userAgent);
  const isAndroid = /Android/.test(navigator.userAgent);
  const chromeIcon = isMacDesktop ? 14 : 16;
  let fullscreen = $state(false);
  // Check for an app update on launch: desktop via the Tauri updater, Android via
  // the GitHub releases check (#145). Both silently no-op if up to date/offline.
  onMount(() => {
    if (isMacDesktop) checkForUpdate();
    else if (isAndroid) checkForUpdateMobile();
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

  function onSwipeStart(e: TouchEvent) {
    if (!mobile || settingsOpen || e.touches.length !== 1) return;
    const t = e.touches[0];
    if (t.clientX <= EDGE || t.clientX >= window.innerWidth - EDGE) return;
    panStart = { x: t.clientX, y: t.clientY, opening: !sidebarOpen };
    panLocked = false;
  }
  function onSwipeMove(e: TouchEvent) {
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
  function onSwipeEnd() {
    if (panStart && panLocked && drawerPan !== null)
      setSidebar(panStart.opening ? drawerPan >= COMMIT - DRAWER_W : drawerPan > -COMMIT);
    panStart = null;
    panLocked = false;
    drawerPan = null;
  }

  let activeVault = $state<string | null>(null);
  // Search palette (#15). `searchInitial` seeds the query when opened from a
  // tag chip in the preview, so the tag is pre-filtered.
  let searchOpen = $state(false);
  let searchInitial = $state("");
  let activePath = $state<string | null>(null);
  let content = $state("");
  let lastLoaded = $state("");

  // Note types (#104). A typed note renders one way only, so it hides the
  // source/preview control and shows its own header actions instead.
  const noteType = $derived(noteTypeOf(content));
  const typeInfo = $derived(noteTypeInfo(noteType));
  const singleView = $derived(!!activePath && typeInfo.singleView);
  const checkedCount = $derived(noteType === "todo" ? countChecked(content) : 0);
  // Which view actually renders. Typed notes are always the editor: a TODO note
  // needs text entry as much as it needs tickable boxes, and forcing it to
  // preview left it with no way to add an item at all (#174). The checkboxes are
  // drawn in the editor instead, by the taskCheckboxes decoration.
  const view = $derived(
    noteType === "todo" ? "todo" : noteType === "journal" ? "journal" : singleView ? "source" : mode,
  );

  // Remove every ticked item. Behind a confirm because a delete propagates to
  // every synced device — there is no undo that would work across sync. The
  // rewrite goes through `content`, so it is an ordinary CRDT-merged edit.
  async function sweepDone() {
    if (!checkedCount) return;
    const plural = checkedCount === 1 ? "item" : "items";
    if (!(await sidebar?.confirmAction(`Remove ${checkedCount} completed ${plural}?`))) return;
    content = sweepChecked(content);
  }

  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  // Per-open id for the editor's {#key}, so switching notes remounts the editor
  // with a fresh document.
  let openToken = $state(0);

  // Editor handle + focus, for the mobile markdown toolbar.
  let editorView = $state<EditorView | undefined>(undefined);
  let editorFocused = $state(false);
  // Whether the soft keyboard is up. The toolbar is anchored to the keyboard, so
  // it must hide when the keyboard is dismissed even if the editor keeps focus.
  let kbOpen = $state(false);

  // ---- Quick edit — mobile tap (opt-in, #33) + desktop double-click (#153) ----
  // Mobile: tapping a previewed note jumps to source + keyboard, and hiding the
  // keyboard returns to preview. Desktop: double-clicking the preview jumps to
  // source at the clicked point, and ESC returns to preview (see onKeydown).
  // quickEditActive marks a source view we entered this way (so we only auto-
  // return for those, not manual toggles).
  let quickEditActive = $state(false);
  let kbWasOpen = false; // the keyboard has been up since this quick edit began
  let focusOnMount = false; // focus the editor once it mounts after the tap
  let quickEditCaret: number | null = null; // source offset for the tapped point
  let quickEditCaretY: number | null = null; // tap's viewport Y, to keep it in place
  let quickEditPin: { pos: number; tapY: number } | null = null; // re-pin once kb opens
  let tapStart: { x: number; y: number; t: number } | null = null;

  // Scroll `view` so the caret's line sits at on-screen height `tapY` (where the
  // preview tap was), but never below the keyboard/toolbar — clamp to the
  // scroller's visible bottom. Used on quick-edit entry and again once the soft
  // keyboard opens, since the keyboard shrinks the editor and would otherwise
  // leave the tapped line behind it (#122).
  function pinQuickEditCaret(view: EditorView, pos: number, tapY: number) {
    const c = view.coordsAtPos(pos);
    if (!c) return;
    const rect = view.scrollDOM.getBoundingClientRect();
    const targetY = Math.min(tapY, rect.bottom - 24);
    view.scrollDOM.scrollTop += c.top - targetY;
  }

  // Caret position under a viewport point, across engines (Chromium/WebKit).
  function caretFromPoint(x: number, y: number): { node: Node; offset: number } | null {
    type DocWithCaret = Document & {
      caretPositionFromPoint?: (x: number, y: number) => { offsetNode: Node; offset: number } | null;
    };
    const d = document as DocWithCaret;
    const p = d.caretPositionFromPoint?.(x, y);
    if (p) return { node: p.offsetNode, offset: p.offset };
    const r = document.caretRangeFromPoint?.(x, y);
    return r ? { node: r.startContainer, offset: r.startOffset } : null;
  }

  // Reduce text to a lowercase alphanumeric stream, mapping every run of other
  // characters (whitespace, punctuation, and — for the source — markdown markers)
  // to a single space. Returns the stream plus a map back to source offsets, so a
  // match in the stream can be translated to a caret position in `content`.
  function normalizeWithMap(src: string): { norm: string; map: number[] } {
    const map: number[] = [];
    let norm = "";
    let pendingSpace = false;
    for (let i = 0; i < src.length; i++) {
      const c = src[i];
      if (/[a-z0-9]/i.test(c)) {
        if (pendingSpace && norm.length) {
          norm += " ";
          map.push(i);
        }
        pendingSpace = false;
        norm += c.toLowerCase();
        map.push(i);
      } else {
        pendingSpace = true;
      }
    }
    return { norm, map };
  }

  // Map a tapped point in the preview to a caret offset in the markdown source.
  // The preview's text has markdown stripped, so we normalize both to a plain
  // alphanumeric stream and find the tapped block's visible text-up-to-caret in
  // the source, landing the caret just after the matched run. Returns null when
  // it can't map (caller then just focuses at the existing position).
  function sourceOffsetFromPoint(x: number, y: number): number | null {
    const root = mainEl?.querySelector<HTMLElement>(".md-preview");
    if (!root) return null;
    const norm1 = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, " ").trim();

    // Resolve the *direct child* block of the preview under the tap, plus the
    // caret node/offset within it. A tap on real text yields a precise in-block
    // caret; only trust it when the walk lands on a direct child of the preview.
    const caret = caretFromPoint(x, y);
    let block: HTMLElement | null = null;
    let caretNode: Node | null = null;
    let caretOffset = 0;
    if (caret && root.contains(caret.node) && caret.node !== root) {
      let b: HTMLElement | null =
        caret.node.nodeType === Node.TEXT_NODE ? caret.node.parentElement : (caret.node as HTMLElement);
      while (b && b.parentElement && b.parentElement !== root) b = b.parentElement;
      if (b && b.parentElement === root) {
        block = b;
        caretNode = caret.node;
        caretOffset = caret.offset;
      }
    }

    // Tap fell in a gap or below the content — common at the bottom of a note,
    // where `caretFromPoint` returns the `.md-preview` container itself and the
    // walk above would climb past the root. Pick the block nearest the tap's Y
    // and land the caret at its *end*, instead of returning null and dropping the
    // caret to the top of the document (#122).
    if (!block) {
      const blocks = Array.from(root.children) as HTMLElement[];
      for (const el of blocks) {
        const rect = el.getBoundingClientRect();
        if (y >= rect.top && y <= rect.bottom) {
          block = el;
          break;
        }
        if (y > rect.bottom) block = el; // last block wholly above the tap
      }
      block ??= blocks[blocks.length - 1] ?? null;
      if (!block) return null;
      caretNode = block;
      caretOffset = block.childNodes.length; // end of the block
    }

    const r = document.createRange();
    r.selectNodeContents(block);
    r.setEnd(caretNode!, caretOffset);
    const prefix = norm1(r.toString()); // tapped block's text up to the caret
    const { norm, map } = normalizeWithMap(content);
    // Locate the tapped block in the source by its *full* text (much more unique
    // than the prefix), then offset within it by the prefix — so a phrase that
    // repeats elsewhere in the note doesn't drag the caret to the wrong place.
    const blockText = norm1(block.textContent ?? "");
    const base = blockText ? norm.indexOf(blockText) : -1;
    const start = base >= 0 ? base : norm.indexOf(prefix);
    if (start < 0) return null;
    const caretNorm = start + prefix.length;
    if (caretNorm <= 0) return null;
    let offset = map[Math.min(caretNorm, map.length) - 1] + 1;
    // The normalized prefix ends on the block's last *alphanumeric* char, so a tap
    // at the end of a line lands `offset` just *before* any trailing punctuation or
    // markup ("writing|!", "Wi-Fi|."). When the prefix covers the whole block (an
    // end-of-line tap — middle taps have a shorter prefix), advance to the end of
    // that source line so the caret sits after those trailing characters (#122).
    if (prefix === blockText) {
      while (offset < content.length && content[offset] !== "\n") offset++;
    }
    return offset;
  }

  // Quick edit is a Markdown-note affordance: tap the preview to jump into
  // source. A typed note (#180/#181) has a single operational mode, so there is
  // nothing to jump to — and letting these run swallowed taps on the checklist's
  // own controls, which is why a TODO note felt dead on mobile (#174 again, from
  // a different direction).
  const quickEditable = $derived(mobile && editorSettings.quickEdit && !singleView);

  function onPreviewPointerDown(e: PointerEvent) {
    if (!(quickEditable && mode === "preview")) return;
    tapStart = { x: e.clientX, y: e.clientY, t: e.timeStamp };
  }
  // A recognized scroll/gesture fires pointercancel (not pointerup); clear the
  // pending tap so the next genuine tap isn't matched against this stale start.
  function onPreviewPointerCancel() {
    tapStart = null;
  }
  function onPreviewPointerUp(e: PointerEvent) {
    const s = tapStart;
    tapStart = null;
    if (!s || !(quickEditable && mode === "preview")) return;
    // A drag (selection/scroll) or long-press isn't a "tap" — leave it in preview.
    if (Math.hypot(e.clientX - s.x, e.clientY - s.y) > 10 || e.timeStamp - s.t > 500) return;
    // Links and task checkboxes have their own tap behavior; don't hijack them.
    if ((e.target as HTMLElement | null)?.closest("a, input")) return;
    // Map the tap to a source caret before we leave preview (DOM is still here).
    quickEditCaret = sourceOffsetFromPoint(e.clientX, e.clientY);
    quickEditCaretY = e.clientY; // so source can keep the tapped line at this height
    quickEditActive = true;
    kbWasOpen = false;
    focusOnMount = true;
    setMode("source");
  }

  // Desktop quick edit (#153): double-click the preview to jump into source at
  // the clicked point. ESC returns to preview (onKeydown). No keyboard on desktop,
  // so unlike mobile there's no auto-return on keyboard-dismiss — the focus-on-
  // mount effect places the caret; quickEditActive just marks it for ESC.
  function onPreviewDblClick(e: MouseEvent) {
    if (mobile || singleView || mode !== "preview" || !activePath) return;
    // Links and task checkboxes have their own behavior; don't hijack them.
    if ((e.target as HTMLElement | null)?.closest("a, input")) return;
    quickEditCaret = sourceOffsetFromPoint(e.clientX, e.clientY);
    quickEditCaretY = null; // desktop: no keyboard to pin the line above
    quickEditActive = true;
    focusOnMount = true;
    setMode("source");
  }

  // Focus the editor once it has mounted from the quick-edit tap, which raises
  // the soft keyboard. (The Editor only exists in source mode, so this can't run
  // inside the tap handler.)
  $effect(() => {
    // Read all three reactive deps up-front and unconditionally. As a single
    // `focusOnMount && mode === "source" && editorView` guard, `&&` short-circuits:
    // on the runs while focusOnMount/mode are still settling during the
    // preview→source swap, `editorView` is never read, so Svelte doesn't track it
    // — and the effect then won't re-run when the freshly-mounted editor *binds*,
    // silently skipping the quick-edit caret placement (the caret then falls to
    // wherever the tap's synthetic click lands in the top-scrolled editor). Read
    // them into locals so all three are always tracked (#122).
    const armed = focusOnMount;
    const inSource = mode === "source";
    const v = editorView;
    if (!(armed && inSource && v)) return;
    focusOnMount = false;
    const caret = quickEditCaret;
    const tapY = quickEditCaretY;
    quickEditCaret = null;
    quickEditCaretY = null;
    v.focus();
    // Place the caret where the user tapped in the preview (#41). Falls back to
    // the editor's existing position when the tap couldn't be mapped.
    if (caret == null) return;
    const pos = Math.max(0, Math.min(caret, v.state.doc.length));
    // Place the caret, then scroll so its line sits at the same on-screen height
    // the tap had in the preview — a continuous transition instead of a jump to
    // the viewport edge (what plain scrollIntoView does). scrollIntoView in place()
    // first renders/reveals the line so coordsAtPos is measurable; pin() then
    // shifts scrollTop to the tap's Y. The keyboard isn't up yet here, so pin()
    // again once it opens (see below) — that reflow shrinks the editor (#122).
    const place = () => v.dispatch({ selection: { anchor: pos }, scrollIntoView: true });
    const pin = () => tapY != null && pinQuickEditCaret(v, pos, tapY);
    place();
    pin();
    if (tapY != null) quickEditPin = { pos, tapY };
    // The tap's synthetic click must be allowed through — it's what focuses the
    // editor and raises the soft keyboard on Android (programmatic focus alone
    // doesn't). But that click *natively* moves the caret to the tapped pixel,
    // which in the freshly-mounted, top-scrolled editor is the wrong offset, and
    // CodeMirror mirrors that DOM change back into its state a few frames later.
    // Re-assert across a short window: each frame, if CM has drifted off our
    // target, put it back and re-pin the scroll; once the synthetic sequence
    // settles this is a no-op. Bails out if we leave the editor (#122).
    let frames = 0;
    const enforce = () => {
      if (!v.dom.isConnected) return;
      if (v.state.selection.main.head !== pos) {
        place();
        pin();
      }
      if (++frames < 20) requestAnimationFrame(enforce);
    };
    requestAnimationFrame(enforce);
  });

  // While a quick edit is active, returning the keyboard to hidden returns to
  // preview — but only after it was actually raised, so we don't bounce back
  // before it appears.
  $effect(() => {
    if (!quickEditActive) return;
    if (kbOpen) {
      kbWasOpen = true;
      // The keyboard opening shrinks the editor (--editor-kb-inset = keyboard +
      // toolbar), which reflows the tapped line — potentially behind the keyboard
      // or the markdown toolbar. Both land over several frames: the keyboard
      // animates open and the toolbar only mounts/measures its height once kbOpen
      // flips. So watch across a longer window and, *only while the caret is
      // occluded* below the scroller's visible bottom, pull it back up to the
      // tap's height clamped above that bottom. Conditional so it catches the late
      // toolbar inset without fighting the user once the caret is already visible
      // (#122, #147).
      const p = quickEditPin;
      const v = editorView;
      if (p && v) {
        quickEditPin = null;
        let n = 0;
        const settle = () => {
          if (!v.dom.isConnected) return;
          const c = v.coordsAtPos(p.pos);
          const bottom = v.scrollDOM.getBoundingClientRect().bottom - 24;
          if (c && c.bottom > bottom) v.scrollDOM.scrollTop += c.top - Math.min(p.tapY, bottom);
          if (++n < 32) requestAnimationFrame(settle);
        };
        requestAnimationFrame(settle);
      }
    } else if (kbWasOpen) {
      quickEditActive = false;
      kbWasOpen = false;
      quickEditPin = null;
      if (mode === "source") setMode("preview");
    }
  });

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
  // editor once it mounts (#50). $state so the `focusOnMount` prop binding tracks it.
  let focusNewNote = $state(false);

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
    if (path !== activePath) handleOpen(activeVault, path);
    if (!fragment) return;
    const sel = `#${CSS.escape(slugify(fragment))}`;
    let done = false;
    for (const d of [0, 60, 150, 300])
      setTimeout(() => {
        if (done) return;
        const el = mainEl?.querySelector(sel);
        if (el) {
          done = true;
          el.scrollIntoView({ behavior: "smooth", block: "start" });
        }
      }, d);
  }

  async function handleOpen(vault: string, path: string, focus = false) {
    clearTimeout(saveTimer);
    resetChrome();
    // Restore the saved scroll only for the note reopened at launch; any other
    // open (or switching notes) starts at the top.
    const restoreRatio =
      !didFirstOpen && initialPath && path === initialPath ? initialScroll : 0;
    didFirstOpen = true;
    activeVault = vault;
    activePath = path;
    session.vault = vault;
    session.path = path;
    session.scroll = restoreRatio;
    // A new note always opens in source mode so the user can type right away.
    if (focus && mode !== "source") {
      mode = "source";
      session.mode = "source";
    }
    focusNewNote = focus;
    // Clear the previous note's text before the (async) read so the new note
    // never briefly shows the old content while readNote resolves (#44). Set
    // lastLoaded too so the autosave effect doesn't treat this as an edit.
    content = "";
    lastLoaded = "";
    content = await readNote(vault, path);
    lastLoaded = content;
    openToken++;
    if (mobile) setSidebar(false);
    await tick();
    // The remounted Editor read focusNewNote via its focusOnMount prop and
    // focused itself; clear the flag so the next (non-new) open doesn't.
    focusNewNote = false;
    applyScrollRestore(mode, restoreRatio);
  }

  function handleVaultChange(vault: string | null) {
    clearTimeout(saveTimer);
    resetChrome();
    activeVault = vault;
    activePath = null;
    content = "";
    lastLoaded = "";
    session.vault = vault;
    session.path = null;
  }

  function closeNote() {
    clearTimeout(saveTimer);
    resetChrome();
    activePath = null;
    content = "";
    lastLoaded = "";
    session.path = null;
  }

  // FAB / Cmd+N: one tap creates and opens an "Untitled" note in the current
  // note's folder (root if none) — no name prompt — so the user can type right
  // away (#85). Creation lives in the sidebar alongside its other dialogs.
  function newNoteHere() {
    sidebar?.newUntitledNote(currentDir);
  }

  // Breadcrumb rename: long-press (mobile) mirrors the double-click (desktop)
  // path; both open the sidebar's rename prompt for the active note (#52).
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
    clearTimeout(saveTimer);
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
  // Import/export run through native dialogs + the backend; surface any failure
  // instead of letting the promise reject silently (#79).
  function reportTransferError(e: unknown) {
    console.error("import/export failed", e);
  }
  async function onImportNote() {
    if (!activeVault) return;
    try {
      const created = await importNoteMd(activeVault, currentDir);
      if (created) handleOpen(activeVault, created);
    } catch (e) {
      reportTransferError(e);
    }
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
    await deletePath(activeVault, activePath, false);
    closeNote();
  }

  // Autosave: debounce content writes 400ms. The filename never changes from
  // content, so a single content-only save is all we need.
  $effect(() => {
    const c = content;
    const v = activeVault;
    const p = activePath;
    // `base` = the text last synced to the backend; it merges base→c against
    // concurrent peer edits so a remote change isn't clobbered (#99).
    const base = lastLoaded;
    if (!v || !p || c === lastLoaded) return;
    clearTimeout(saveTimer);
    saveTimer = setTimeout(async () => {
      // Advance the base *before* awaiting: if the user types again while this
      // write is in flight, the next save must merge against the text we just
      // sent — not this same stale `base` — or the 3-way merge sees both sides
      // diverging from an old ancestor and injects spurious conflict markers.
      lastLoaded = c;
      try {
        await writeNote(v, p, c, base);
      } catch (e) {
        // Write failed: restore the prior base so the effect retries this edit.
        if (lastLoaded === c) lastLoaded = base;
        throw e;
      }
    }, 400);
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
    window.addEventListener("scroll", onAnyScroll, true);
    window.addEventListener("touchstart", onSwipeStart, { capture: true, passive: true });
    window.addEventListener("touchmove", onSwipeMove, { capture: true, passive: false });
    window.addEventListener("touchend", onSwipeEnd, { capture: true });
    window.addEventListener("touchcancel", onSwipeEnd, { capture: true });

    // Detect the soft keyboard from the visual viewport. It shrinks (relative to
    // the tallest height we've seen with no keyboard) whenever the keyboard is
    // up, in both adjustResize and adjustPan modes — so this is layout-agnostic.
    const vv = window.visualViewport;
    let maxVvH = vv ? vv.height : window.innerHeight;
    const onVv = () => {
      if (!vv) return;
      maxVvH = Math.max(maxVvH, vv.height);
      kbOpen = vv.height < maxVvH - 150;
    };
    if (vv) {
      onVv();
      vv.addEventListener("resize", onVv);
      vv.addEventListener("scroll", onVv);
    }

    onVaultChanged(async (vaultId) => {
      const vault = activeVault;
      const path = activePath;
      if (vaultId !== vault || !path || content !== lastLoaded) return;
      const fresh = await readNote(vault, path);
      // The open note may have changed while readNote was in flight — e.g.
      // creating a note fires vault-changed, and opening it swaps activePath
      // mid-read. Without this guard the in-flight read of the *previous* note
      // lands in the new note's editor, so a just-created note briefly shows the
      // old note's text until you navigate away and back (#123, desktop timing).
      if (vault !== activeVault || path !== activePath) return;
      if (fresh === lastLoaded) return;
      // A remote key update can arrive before its content blob finishes
      // downloading; read_note then returns empty (unwrap_or_default). Don't
      // wipe a non-empty note on that transient — the blob-complete event fires
      // next with the real content. Without this, the open editor is cleared and
      // refilled, collapsing the caret/scroll to the top (issue #25).
      if (fresh === "" && lastLoaded !== "") return;
      content = fresh;
      lastLoaded = fresh;
    }).then((u) => (unlisten = u));
    return () => {
      mq.removeEventListener("change", onMq);
      window.removeEventListener("keydown", onKeydown);
      window.removeEventListener("scroll", onAnyScroll, true);
      window.removeEventListener("touchstart", onSwipeStart, { capture: true });
      window.removeEventListener("touchmove", onSwipeMove, { capture: true });
      window.removeEventListener("touchend", onSwipeEnd, { capture: true });
      window.removeEventListener("touchcancel", onSwipeEnd, { capture: true });
      vv?.removeEventListener("resize", onVv);
      vv?.removeEventListener("scroll", onVv);
      unlisten?.();
    };
  });

  // Global hotkeys. Mod = Cmd (macOS) / Ctrl (elsewhere). These complement the
  // editor's own text-formatting shortcuts, which CodeMirror handles internally.
  function onKeydown(e: KeyboardEvent) {
    // ESC leaves a desktop quick edit (#153), returning to preview. Gated on
    // quickEditActive so it only undoes a double-click-to-edit, not a manual
    // source view (where ESC belongs to CodeMirror — closing autocomplete, etc.).
    if (e.key === "Escape") {
      if (!mobile && mode === "source" && quickEditActive && activePath) {
        e.preventDefault();
        editorView?.contentDOM.blur();
        setMode("preview");
      }
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
        setMode(mode === "source" ? "preview" : "source");
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
    class="flex shrink-0 items-center justify-between gap-3 border-b border-border {isMacDesktop
      ? 'min-h-0 px-2'
      : 'min-h-12 px-3 pb-2'} {mobile
      ? 'absolute inset-x-0 top-0 z-10 bg-background'
      : 'bg-secondary/40'}"
    style="padding-top:calc(env(safe-area-inset-top) + {isMacDesktop
      ? '0.25rem'
      : '0.5rem'});{isMacDesktop ? 'padding-bottom:0.25rem;' : ''}{isMacDesktop && !fullscreen
      ? 'padding-left:78px;'
      : ''}{mobile
      ? `transition:transform 200ms ease, opacity 200ms ease;transform:translateY(${chromeHidden
          ? '-100%'
          : '0'});opacity:${chromeHidden ? 0 : 1};`
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
                >{/if}{#if i === parts.length - 1}<!-- The filename: double-click
                (desktop) or long-press (mobile) to rename the open note (#52). -->
                <!-- svelte-ignore a11y_no_static_element_interactions -->
                <span
                  class="cursor-text text-foreground"
                  title="Double-click or long-press to rename"
                  ondblclick={() => sidebar?.renameActive()}
                  onpointerdown={crumbPressStart}
                  onpointerup={crumbPressEnd}
                  onpointerleave={crumbPressEnd}
                  oncontextmenu={(e) => e.preventDefault()}>{seg}</span
                >{:else}<span class="text-muted-foreground/60">{seg}</span>{/if}
            {/each}
          </bdo>
        {/if}
      </span>
    </div>

    <div data-tauri-drag-region class="flex items-center gap-1.5">
      {#if noteType === "todo" && checkedCount}
        <!-- Sweep completed items (#104). Only shown when there's something to
             sweep, so it doesn't sit there as dead chrome. -->
        <button
          type="button"
          class="flex items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-muted hover:text-foreground {isMacDesktop
            ? 'p-1'
            : 'p-2'}"
          aria-label="Remove completed items"
          title="Remove {checkedCount} completed item{checkedCount === 1 ? '' : 's'}"
          onclick={sweepDone}
        >
          <BrushCleaning size={chromeIcon} />
        </button>
      {/if}
      <!-- Single toggle: click anywhere flips Source<->Preview; active half is lit.
           Hidden for typed notes (#104), which have exactly one view. -->
      {#if !singleView}
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
          onopen={handleOpen}
          onvaultchange={handleVaultChange}
          ontree={(t) => (tree = t)}
        />
      </div>
    </aside>

    <main
      bind:this={mainEl}
      class="min-w-0 flex-1 {view === 'journal' ? 'flex flex-col overflow-hidden' : 'overflow-auto'}"
      style={mobile
        ? `padding-top:${view === "preview" || view === "todo" || view === "journal" ? headerH : 0}px;`
        : ""}
      onpointerdown={onPreviewPointerDown}
      onpointerup={onPreviewPointerUp}
      onpointercancel={onPreviewPointerCancel}
      ondblclick={onPreviewDblClick}
    >
      {#if !activePath}
        <div class="flex h-full flex-col items-center justify-center gap-3 text-muted-foreground">
          <NotebookPen size={40} class="opacity-30" />
          <p class="text-sm">Select or create a note.</p>
        </div>
      {:else if view === "todo"}
        <TodoList bind:value={content} />
      {:else if view === "journal"}
        <JournalView bind:value={content} {mobile} {notePaths} ontag={openTagSearch} oninternallink={openInternalLink} />
      {:else if view === "preview"}
        <Preview
          bind:value={content}
          {notePaths}
          oninternallink={openInternalLink}
          ontag={openTagSearch}
          loadNote={(path) => readNote(activeVault!, path)}
        />
      {:else}
        {#key openToken}
          <Editor
            bind:value={content}
            bind:view={editorView}
            bind:focused={editorFocused}
            {notePaths}
            focusOnMount={focusNewNote}
            ontag={openTagSearch}
          />
        {/key}
      {/if}
    </main>
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

<!-- Mobile: markdown toolbar anchored above the soft keyboard while editing -->
{#if mobile && mode === "source" && activePath && editorFocused && editorView && kbOpen}
  <MarkdownToolbar view={editorView} />
{/if}

<SearchPalette
  bind:open={searchOpen}
  vault={activeVault}
  initial={searchInitial}
  onopen={(path) => activeVault && handleOpen(activeVault, path)}
/>

<SettingsSheet
  bind:open={settingsOpen}
  {activePath}
  {folders}
  {currentDir}
  onmove={moveNote}
  onduplicate={duplicateNote}
  oncopy={copyContents}
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
