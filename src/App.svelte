<script lang="ts">
  import { onMount, tick } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import type { EditorView } from "@codemirror/view";
  import Editor from "$lib/components/editor/Editor.svelte";
  import Preview from "$lib/components/editor/Preview.svelte";
  import JournalView from "$lib/components/editor/JournalView.svelte";
  import Sidebar from "$lib/components/sidebar/Sidebar.svelte";
  import TabStrip from "$lib/components/TabStrip.svelte";
  import MarkdownToolbar from "$lib/components/editor/MarkdownToolbar.svelte";
  import SettingsSheet from "$lib/components/SettingsSheet.svelte";
  import SearchPalette from "$lib/components/SearchPalette.svelte";
  import { checkForUpdate, checkForUpdateMobile } from "$lib/updater";
  import Fab from "$lib/components/Fab.svelte";
  import {
    readNote,
    writeNote,
    createNote,
    renamePath,
    deletePath,
    shareNote,
    onVaultChanged,
    onBackgroundSyncChanged,
    type TreeNode,
  } from "$lib/vault";
  import { isAndroidApp, isMacApp } from "$lib/platform";
  import { markAppScroll, appScrolling } from "$lib/app-scroll";
  import { softKeyboard } from "$lib/soft-keyboard";
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
  import { noteTypeInfo, noteTypeOf } from "$lib/note-type";
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
      markAppScroll();
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
  // Scrolls the app makes itself never move the chrome, however close behind a
  // real gesture they land — see $lib/app-scroll, which Preview and Editor mark
  // through as well.
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
        // Re-baseline on every event, including the ones ignored below — a
        // programmatic jump left in the baseline would be charged to whatever
        // gesture came next, and hide the chrome on a scroll that never moved.
        lastChromeTop = top;
        // Back at the top, the chrome comes back whatever moved the scroller.
        if (top < 8) chromeHidden = false;
        // Anything else only moves the chrome if the user was scrolling just
        // now — which includes the fling after they let go, but never our own
        // scrolling, however close behind theirs it lands.
        else if (userScrolling() && !appScrolling()) {
          if (delta > 6) chromeHidden = true;
          else if (delta < -6) chromeHidden = false;
        }
      }
    }
    clearTimeout(scrollSaveTimer);
    scrollSaveTimer = setTimeout(() => {
      if (activePath) session.scroll = scrollRatio(mode);
    }, 150);
  }
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
  // Only the active tab holds a buffer: switching tabs flushes the pending save
  // and re-reads the note it lands on (see syncActive). `loadedVault`/
  // `loadedPath` say which note `content` belongs to — which is *not* always
  // the active one, since the read is async and the flush of the outgoing note
  // has to know where to send its text.
  let content = $state("");
  let lastLoaded = $state("");
  let loadedVault: string | null = null;
  let loadedPath: string | null = null;

  // Note types (#104). A typed note renders one way only, so it hides the
  // source/preview control and shows its own header actions instead.
  const noteType = $derived(noteTypeOf(content));
  const typeInfo = $derived(noteTypeInfo(noteType));
  const singleView = $derived(!!activePath && typeInfo.singleView);
  // Which view actually renders. A typed note is always the editor: it has
  // exactly one operational mode, drawn by its own component.
  const view = $derived(noteType === "journal" ? "journal" : singleView ? "source" : mode);

  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  // Per-open id for the editor's {#key}, so switching notes remounts the editor
  // with a fresh document.
  let openToken = $state(0);

  // Editor handle + focus, for the mobile markdown toolbar.
  let editorView = $state<EditorView | undefined>(undefined);
  // Journal handle, so a chunk being edited can be committed before we leave
  // the note it belongs to (see flushSave).
  let journalView = $state<JournalView | undefined>(undefined);
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
    markAppScroll();
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
  // nothing to jump to — and letting these run would swallow taps on the note's
  // own controls instead.
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
          if (c && c.bottom > bottom) {
            markAppScroll();
            v.scrollDOM.scrollTop += c.top - Math.min(p.tapY, bottom);
          }
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
    if (path !== activePath) openNote(activeVault, path);
    if (!fragment) return;
    const sel = `#${CSS.escape(slugify(fragment))}`;
    let done = false;
    for (const d of [0, 60, 150, 300])
      setTimeout(() => {
        if (done) return;
        const el = mainEl?.querySelector(sel);
        if (el) {
          done = true;
          // Smooth, so it emits scroll events for a few hundred ms — claim all
          // of them, or the tail of our own jump hides the chrome (#250).
          markAppScroll(700);
          el.scrollIntoView({ behavior: "smooth", block: "start" });
        }
      }, d);
  }

  // ---- Tabs (#169) ----
  //
  // The tab list is the session store's; this side owns the buffer for the
  // *active* tab only. Everything that changes which tab is active ends in
  // syncActive(), which is the one place the buffer is swapped.

  /**
   * Bring the buffer in line with the active tab: drop the outgoing text and
   * read the note we've landed on. Callers park the outgoing tab (flushSave)
   * *before* they change which tab is active, since that is the last moment its
   * pending edit and scroll position can be written to the right note.
   *
   * Every state change up to the first await is synchronous on purpose. The
   * autosave effect keys off (activePath, content), and if it ever observed the
   * outgoing note's text against the incoming note's path it would write one
   * over the other.
   */
  async function syncActive() {
    const v = session.vault;
    const p = session.path;
    if (v === loadedVault && p === loadedPath) return;
    clearTimeout(saveTimer);
    loadedVault = v;
    loadedPath = p;
    // Clear before the (async) read so the new note never briefly shows the old
    // content (#44). lastLoaded too, so the autosave effect sees no edit.
    content = "";
    lastLoaded = "";
    // Whatever the backend last refused belonged to the note we just left, and
    // holding on to it would block saving that same text there again.
    rejectedEdit = null;
    saveBlocked = null;
    // A quick edit belongs to the note it started in.
    quickEditActive = false;
    resetChrome();
    if (!v || !p) return;
    const text = await readNote(v, p);
    // We may have moved on again while the read was in flight — e.g. creating a
    // note fires vault-changed, and opening it swaps tabs mid-read (#123).
    if (v !== loadedVault || p !== loadedPath) return;
    content = text;
    lastLoaded = text;
    openToken++;
    await tick();
    // The remounted Editor read focusNewNote via its focusOnMount prop and
    // focused itself; clear the flag so the next (non-new) open doesn't.
    focusNewNote = false;
    // Each tab remembers where it was left, so this restores on every switch,
    // not just at launch.
    applyScrollRestore(mode, session.scroll);
  }

  /**
   * Park the active tab before its buffer is dropped: write where it was left,
   * and land any edit the 400ms debounce hasn't sent yet. Returns false if that
   * write failed — the caller then leaves the note open rather than throwing
   * the text away, and the #253 banner says why (its "Save a copy" is the way
   * out).
   *
   * Runs while the tab is still the active one, which is what makes the two
   * session writes land on it rather than on the tab we're moving to.
   */
  async function flushSave(): Promise<boolean> {
    clearTimeout(saveTimer);
    // A journal chunk mid-edit lives in the view's own state until something
    // finishes it — normally the blur from clicking elsewhere. A hotkey moves
    // no focus, so ask for it explicitly, while this is still the open note.
    journalView?.commitPending();
    // The debounced scroll-save may not have fired yet, and afterwards it would
    // credit this tab's position to the next one.
    clearTimeout(scrollSaveTimer);
    if (loadedPath) session.scroll = scrollRatio(mode);
    const v = loadedVault;
    const p = loadedPath;
    const c = content;
    const base = lastLoaded;
    if (!v || !p || c === base) return true;
    if (rejectedEdit && rejectedEdit.path === p && rejectedEdit.content === c) return false;
    lastLoaded = c;
    if (await saveNote(v, p, c, base)) return true;
    // Restore the base so the debounced save retries this edit, exactly as it
    // does when a debounced write fails.
    if (lastLoaded === c) lastLoaded = base;
    return false;
  }

  async function openNote(
    vault: string,
    path: string,
    opts: { focus?: boolean; pin?: boolean; newTab?: boolean } = {},
  ) {
    // A path that has left the tabs was just renamed or deleted (those callers
    // flush before they touch the vault), so there is nothing left to send.
    const stale = !!loadedPath && !session.tabs.some((t) => t.path === loadedPath);
    if (!stale && path !== loadedPath && !(await flushSave())) return;
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
    await syncActive();
  }

  async function selectTab(i: number) {
    if (i === session.active) return;
    if (!(await flushSave())) return;
    session.activate(i);
    await syncActive();
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
    await selectTab(i);
    if (session.active === i && session.tabs[i]?.path === t.path) sidebar?.renameActive();
  }

  async function closeTab(i: number) {
    // Only the active tab holds a buffer, so only closing that one can lose an
    // edit that hasn't landed yet.
    if (i === session.active && !(await flushSave())) return;
    session.close(i);
    await syncActive();
  }

  // A note (or a folder of them) renamed anywhere — the sidebar's rename, a
  // drag in the tree, or Move in the settings sheet. Tabs follow it, including
  // background ones, so none is left pointing at a path that no longer exists.
  async function notesRenamed(from: string, to: string, isDir: boolean) {
    if (loadedPath === from) loadedPath = to;
    else if (isDir && loadedPath?.startsWith(from + "/"))
      loadedPath = to + loadedPath.slice(from.length);
    session.renamed(from, to, isDir);
    await syncActive();
  }

  // Deleted, likewise: its tabs go. Nothing is flushed — the note is gone.
  async function notesRemoved(path: string, isDir: boolean) {
    session.removed(path, isDir);
    await syncActive();
  }

  async function handleVaultChange(vault: string | null) {
    // Only worth flushing when we're leaving a vault we had open; the launch
    // sequence reports the restored vault as a "change" with nothing loaded.
    if (loadedPath) await flushSave();
    // Setting the vault closes the tabs when it differs from the one they were
    // opened in — and keeps them when it doesn't, which is the launch restore.
    session.vault = vault;
    needsPrune = true;
    await syncActive();
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
    await syncActive();
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
    const path = activePath;
    await deletePath(activeVault, path, false);
    await notesRemoved(path, false);
  }

  // The edit a save last failed on, for any reason the backend didn't mark as
  // worth retrying — the note was deleted elsewhere (#251), but equally a dead
  // IPC channel or a closed vault. Rolling the base back is what makes the
  // effect retry, so re-attempting a failure that can't resolve is a 400ms loop
  // that never ends. Keyed to the text, not just the path: a further edit is
  // worth one more attempt in case whatever it was has cleared, but the same
  // rejected text is not. The typed text stays on screen either way — it just
  // isn't being saved, which is why #253 wants to say so.
  let rejectedEdit: { path: string; content: string } | null = null;
  // The same fact, in a form the markup can react to. Kept separate from
  // `rejectedEdit` on purpose: that one is read inside the autosave effect, and
  // making it reactive would put a write to it back into that effect's own
  // dependencies — which is how the retry loop got here in the first place.
  let saveBlocked = $state<string | null>(null);

  /// Put the text somewhere safe when its own note won't take it. Creates a
  /// sibling — `createNote` de-duplicates the name — writes the buffer to it,
  /// and opens it, so the note that is gone stays gone and the words don't.
  async function saveRejectedCopy() {
    const v = activeVault;
    const p = activePath;
    if (!v || !p) return;
    const text = content;
    const slash = p.lastIndexOf("/");
    const dir = slash === -1 ? "" : p.slice(0, slash);
    const stem = p.slice(slash + 1).replace(/\.md$/, "");
    const name = `${stem} (recovered).md`;
    try {
      const created = await createNote(v, dir ? `${dir}/${name}` : name);
      await writeNote(v, created, text, "");
      saveBlocked = null;
      rejectedEdit = null;
      // The words are safe in the copy now, so the buffer has nothing pending —
      // say so, or opening the copy would try (and fail) to flush it back to
      // the note that wouldn't take it, and refuse to switch.
      lastLoaded = content;
      await openNote(v, created, { focus: true });
    } catch (e) {
      console.error("could not save a copy", e);
    }
  }

  /**
   * Write one note, and own what the failure means (#251/#253). Returns whether
   * it landed; the caller decides what to do with the buffer, since the
   * debounced save and the flush-on-tab-switch answer that differently.
   */
  async function saveNote(v: string, p: string, c: string, base: string): Promise<boolean> {
    try {
      await writeNote(v, p, c, base);
      rejectedEdit = null;
      saveBlocked = null;
      return true;
    } catch (e) {
      // Retrying is the exception, not the rule: only a write that says it is
      // waiting on a sync will succeed if we just try again. Anything else
      // fails identically every time, and since the caller rolls the base back
      // to retry, that would be a 400ms loop with no end. Latching on "not
      // retryable" rather than listing the failures that aren't keeps a future
      // error from reintroducing that loop.
      if (!String(e).includes("still syncing")) {
        rejectedEdit = { path: p, content: c };
        // Say so, rather than leaving the note looking saved (#253). The
        // deleted case is worth naming; anything else is honest but vague.
        saveBlocked = String(e).includes("no longer exists")
          ? "This note was deleted on another device, so your changes aren't being saved."
          : String(e).includes("can't be read")
            ? "This note's saved content can't be read, so your changes aren't being saved."
            : "Your changes aren't being saved.";
      }
      console.error("save failed", e);
      return false;
    }
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
    if (!v || !p || c === base) return;
    // Editing a preview tab pins it — the note is yours now, so the next click
    // in the sidebar opens beside it instead of replacing it (#169). Idempotent
    // once pinned, which is what keeps this out of its own dependencies.
    session.pinActive();
    // Cancel any queued save first: returning with one still pending would let
    // it write text the editor has since moved on from.
    clearTimeout(saveTimer);
    if (rejectedEdit && rejectedEdit.path === p && rejectedEdit.content === c) return;
    saveTimer = setTimeout(async () => {
      // Advance the base *before* awaiting: if the user types again while this
      // write is in flight, the next save must merge against the text we just
      // sent — not this same stale `base` — or the 3-way merge sees both sides
      // diverging from an old ancestor and injects spurious conflict markers.
      lastLoaded = c;
      // Restore the prior base so the effect retries this edit — the base stays
      // truthful, which is the whole point of #251.
      if (!(await saveNote(v, p, c, base)) && lastLoaded === c) lastLoaded = base;
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
      window.removeEventListener("wheel", onWheelGesture, { capture: true });
      vv?.removeEventListener("resize", onVv);
      vv?.removeEventListener("scroll", onVv);
      window.removeEventListener("resize", onVv);
      window.removeEventListener("orientationchange", onVv);
      unlisten?.();
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
        onclick={() => setMode(mode === "source" ? "preview" : "source")}
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

    <!-- A save that isn't happening, said out loud (#253). Fixed rather than in
         flow so it can't shift the editor's layout, and offset below the header
         so it clears the floating mobile chrome. -->
    {#if saveBlocked}
      <div
        class="fixed left-1/2 z-40 flex max-w-[min(30rem,calc(100vw-2rem))] -translate-x-1/2 items-center gap-3 rounded-2xl border border-border bg-popover px-4 py-2 text-sm shadow-lg"
        style="top:calc({headerH}px + 0.5rem);"
        role="status"
      >
        <!-- Wraps rather than truncates: the half that matters is the end of
             the sentence, which is exactly what an ellipsis eats. -->
        <span class="min-w-0 text-muted-foreground">{saveBlocked}</span>
        <button
          type="button"
          class="shrink-0 rounded-full bg-primary px-3 py-1 text-xs font-medium text-primary-foreground"
          onclick={saveRejectedCopy}
        >
          Save a copy
        </button>
        <button
          type="button"
          class="shrink-0 rounded-full p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
          aria-label="Dismiss"
          onclick={() => (saveBlocked = null)}
        >
          <X size={14} />
        </button>
      </div>
    {/if}

    <main
      bind:this={mainEl}
      class="min-w-0 flex-1 {view === 'journal' ? 'flex flex-col overflow-hidden' : 'overflow-auto'}"
      style={mobile
        ? `padding-top:${view === "preview" || view === "journal" ? headerH : 0}px;`
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
      {:else if view === "journal"}
        <JournalView bind:this={journalView} bind:value={content} {mobile} {kbOpen} {notePaths} ontag={openTagSearch} oninternallink={openInternalLink} />
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
  onopen={(path) => activeVault && openNote(activeVault, path)}
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
