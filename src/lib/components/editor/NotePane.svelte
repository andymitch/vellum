<script lang="ts">
  // One open note: its text, its saving, its scroll, and whichever of the three
  // views is rendering it. Everything here is scoped to a single note, so the
  // host can have two of these side by side (#169) — which is why the buffer
  // and the autosave live here rather than in App.svelte.
  //
  // The pane is told which note to show (`vault`/`path`) and reports what only
  // it can know back up: the view mode it was toggled into, where it was
  // scrolled, whether the chrome should hide, and that the note has been
  // edited. The host owns the tab list and decides what to do with all of that.
  import { onMount, tick } from "svelte";
  import { EditorView } from "@codemirror/view";
  import Editor from "$lib/components/editor/Editor.svelte";
  import Preview from "$lib/components/editor/Preview.svelte";
  import JournalView from "$lib/components/editor/JournalView.svelte";
  import { readNote, writeNote, createNote, onVaultChanged } from "$lib/vault";
  import { markAppScroll, appScrolling } from "$lib/app-scroll";
  import { slugify } from "$lib/slug";
  import { editorSettings } from "$lib/editor-settings.svelte";
  import { noteTypeInfo, noteTypeOf, type NoteType } from "$lib/note-type";
  import MarkdownToolbar from "$lib/components/editor/MarkdownToolbar.svelte";
  import { X } from "@lucide/svelte";

  type Mode = "source" | "preview";

  let {
    vault,
    path,
    mode,
    scroll = 0,
    mobile = false,
    kbOpen = false,
    headerH = 0,
    notePaths = [],
    focusNew = $bindable(false),
    // The open note's type (#104), so the host can hide the source/preview
    // control for a note that has only one view.
    noteType = $bindable<NoteType>("markdown"),
    // Whether the user was scrolling a moment ago — the host owns the window's
    // touch/wheel listeners, and only a scroll that follows real input may move
    // the chrome (#248).
    userScrolling,
    onmode,
    onscrollratio,
    onchrome,
    onedit,
    onopen,
    ontag,
    oninternallink,
  }: {
    vault: string;
    path: string;
    mode: Mode;
    scroll?: number;
    mobile?: boolean;
    kbOpen?: boolean;
    headerH?: number;
    notePaths?: string[];
    focusNew?: boolean;
    noteType?: NoteType;
    userScrolling: () => boolean;
    /// This pane was toggled between source and preview.
    onmode?: (m: Mode) => void;
    /// Where the pane is scrolled, as a 0..1 ratio (debounced, and on parking).
    onscrollratio?: (ratio: number) => void;
    /// Whether the mobile chrome should be hidden, by this pane's scrolling.
    onchrome?: (hidden: boolean) => void;
    /// The note has a local edit — enough to pin a preview tab (#169).
    onedit?: () => void;
    /// Open another note: the recovered copy a refused save was written to.
    onopen?: (path: string, opts?: { focus?: boolean }) => void;
    ontag?: (tag: string) => void;
    oninternallink?: (path: string, fragment?: string) => void;
  } = $props();

  // The editor handle and whether it has focus. Kept here rather than bound out
  // to the host: Editor clears the handle as it tears down, and carrying that
  // write up through a second binding lands it mid-flush in the host's template
  // — which Svelte refuses (state_unsafe_mutation), aborting the same flush
  // that places the quick-edit caret (#122). The markdown toolbar that needs
  // them is rendered here for the same reason.
  let editorView = $state<EditorView | undefined>(undefined);
  let editorFocused = $state(false);

  // Which note the buffer holds, which is not always the note in the props: the
  // read is async, and a flush of the outgoing note has to know where to send
  // its text.
  let content = $state("");
  let lastLoaded = $state("");
  let loadedVault: string | null = null;
  let loadedPath: string | null = null;

  const typeInfo = $derived(noteTypeInfo(noteType));
  const singleView = $derived(typeInfo.singleView);
  // Which view actually renders. A typed note is always the editor: it has
  // exactly one operational mode, drawn by its own component.
  const view = $derived(noteType === "journal" ? "journal" : singleView ? "source" : mode);
  $effect(() => {
    noteType = noteTypeOf(content);
  });

  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  // Per-open id for the editor's {#key}, so switching notes remounts the editor
  // with a fresh document.
  let openToken = $state(0);
  // Journal handle, so a chunk being edited can be committed before the note is
  // parked (see flush).
  let journalView = $state<JournalView | undefined>(undefined);
  // The chrome-hide baseline, and the scroller it was measured on.
  let lastChromeTop = 0;
  let lastScroller: HTMLElement | null = null;
  function resetChrome() {
    onchrome?.(false);
    lastChromeTop = 0;
  }

  /// The scroller the host's chrome logic should watch — whichever view is up.
  export function scroller(): HTMLElement | null {
    return scrollerFor(mode);
  }
  // A `[[note#heading]]` jump that is still trying to land. It suppresses the
  // note's remembered scroll position (see load): the two would otherwise race,
  // and the restore — firing up to 400ms after the note loads — would win and
  // leave the heading off-screen.
  let pendingAnchor: string | null = null;

  /// Scroll to a `[[note#heading]]` target. Headings get their slug ids after
  /// Preview renders, so retry across a few frames until the anchor exists (#45).
  export function scrollToAnchor(fragment: string) {
    const sel = `#${CSS.escape(slugify(fragment))}`;
    const want = slugify(fragment);
    pendingAnchor = fragment;
    let done = false;
    const attempts = [0, 60, 150, 300, 600];
    for (const d of attempts)
      setTimeout(() => {
        // Whether it landed or the note simply has no such heading, stop
        // suppressing the remembered scroll once the attempts run out.
        if (d === attempts[attempts.length - 1] && pendingAnchor === fragment)
          pendingAnchor = null;
        if (done) return;
        const el = mainEl?.querySelector(sel);
        if (el) {
          done = true;
          // Smooth, so it emits scroll events for a few hundred ms — claim all
          // of them, or the tail of our own jump hides the chrome (#250).
          pendingAnchor = null;
          markAppScroll(700);
          el.scrollIntoView({ behavior: "smooth", block: "start" });
          return;
        }
        // Source mode has no rendered heading to find — the note remembers the
        // view it was left in (#169), so a link into a note last read in source
        // lands here. Find the heading in the text instead and put the caret on
        // it, which is the same "take me to that section" the preview gives.
        const v = editorView;
        if (!v) return;
        let at = -1;
        let pos = 0;
        for (const line of content.split("\n")) {
          const heading = /^#{1,6}\s+(.*)$/.exec(line);
          if (heading && slugify(heading[1]) === want) {
            at = pos;
            break;
          }
          pos += line.length + 1;
        }
        if (at < 0) return;
        done = true;
        pendingAnchor = null;
        markAppScroll(700);
        v.dispatch({
          selection: { anchor: at },
          effects: EditorView.scrollIntoView(at, { y: "start" }),
        });
      }, d);
  }
  /// Copy the note's text (the settings sheet's Copy contents).
  export async function copyContents() {
    try {
      await navigator.clipboard.writeText(content);
    } catch {
      /* clipboard may be unavailable */
    }
  }
  /// ESC leaves a desktop quick edit, returning to preview. Reports whether it
  /// had one to leave, so the host can let ESC through to CodeMirror otherwise.
  export function escapeQuickEdit(): boolean {
    if (mobile || mode !== "source" || !quickEditActive) return false;
    editorView?.contentDOM.blur();
    setMode("preview");
    return true;
  }
  /// Toggle this pane's view mode (the header control, and Cmd+P).
  export function toggleMode() {
    setMode(mode === "source" ? "preview" : "source");
  }
  /// The note (or a folder above it) was renamed: follow it without re-reading,
  /// so the editor isn't torn down and rebuilt for a path change.
  export function renamed(from: string, to: string, isDir: boolean) {
    if (loadedPath === from) loadedPath = to;
    else if (isDir && loadedPath?.startsWith(from + "/"))
      loadedPath = to + loadedPath.slice(from.length);
  }

  // The note to show changed — load it. Keyed on the props, so the host only
  // has to hand this pane a different path.
  $effect(() => {
    const v = vault;
    const p = path;
    if (v === loadedVault && p === loadedPath) return;
    void load();
  });

  // Pull remote edits into this pane's note. A peer's write (or the blob
  // finishing download) emits vault-changed; re-read it. Skip if there are
  // unsaved local edits (content !== lastLoaded) so typing isn't clobbered.
  onMount(() => {
    let unlisten: (() => void) | undefined;
    onVaultChanged(async (vaultId) => {
      const v = loadedVault;
      const p = loadedPath;
      if (vaultId !== v || !v || !p || content !== lastLoaded) return;
      const fresh = await readNote(v, p);
      // The note may have changed while readNote was in flight — e.g. creating
      // a note fires vault-changed, and opening it swaps this pane's note
      // mid-read. Without this guard the in-flight read of the *previous* note
      // lands in the new note's editor (#123, desktop timing).
      if (v !== loadedVault || p !== loadedPath) return;
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
    return () => unlisten?.();
  });

  // The chrome-hide decision is this pane's to make: it watches its own
  // scroller. A capturing window listener sees scroll from either view's
  // scroller (scroll doesn't bubble, but it is observable in the capture
  // phase); events from anywhere else are not ours.
  onMount(() => {
    const onScrollCapture = (e: Event) => {
      const el = scrollerFor(mode);
      if (!el || (e.target !== el && e.target !== document)) return;
      onAnyScroll();
    };
    window.addEventListener("scroll", onScrollCapture, true);
    return () => window.removeEventListener("scroll", onScrollCapture, true);
  });
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
    onmode?.(m);
    onscrollratio?.(ratio);
    await tick();
    // A quick-edit tap positions source's scroll itself (caret pinned to the tap's
    // on-screen height, see the focus effect); ratio-restore would fight that jump.
    if (!(m === "source" && quickEditActive)) applyScroll(m, ratio);
  }


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
        onchrome?.(false);
        lastChromeTop = top;
      } else {
        const delta = top - lastChromeTop;
        // Re-baseline on every event, including the ones ignored below — a
        // programmatic jump left in the baseline would be charged to whatever
        // gesture came next, and hide the chrome on a scroll that never moved.
        lastChromeTop = top;
        // Back at the top, the chrome comes back whatever moved the scroller.
        if (top < 8) onchrome?.(false);
        // Anything else only moves the chrome if the user was scrolling just
        // now — which includes the fling after they let go, but never our own
        // scrolling, however close behind theirs it lands.
        else if (userScrolling() && !appScrolling()) {
          if (delta > 6) onchrome?.(true);
          else if (delta < -6) onchrome?.(false);
        }
      }
    }
    clearTimeout(scrollSaveTimer);
    scrollSaveTimer = setTimeout(() => {
      if (loadedPath) onscrollratio?.(scrollRatio(mode));
    }, 150);
  }

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
    if (mobile || singleView || mode !== "preview" || !path) return;
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


  /**
   * Bring the buffer in line with the note this pane was given: drop the
   * outgoing text and read the new one. The host parks the pane (flush())
   * before it changes the props, since that is the last moment a pending edit
   * and scroll position can be written to the note they belong to.
   *
   * Every state change up to the first await is synchronous on purpose. The
   * autosave effect keys off (activePath, content), and if it ever observed the
   * outgoing note's text against the incoming note's path it would write one
   * over the other.
   */
  async function load() {
    const v = vault;
    const p = path;
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
    focusNew = false;
    // Each tab remembers where it was left, so this restores on every switch,
    // not just at launch — unless we were opened to jump at a heading, which is
    // a more specific request than "where you were".
    if (!pendingAnchor) applyScrollRestore(mode, scroll);
  }


  /**
   * Park this pane before its buffer is dropped: report where it was left,
   * and land any edit the 400ms debounce hasn't sent yet. Returns false if that
   * write failed — the host then leaves the note open rather than throwing the
   * text away, and the #253 banner says why (its "Save a copy" is the way out).
   *
   * Runs before the props change, which is what makes the scroll report land on
   * the note it came from rather than the one we're moving to.
   */
  export async function flush(): Promise<boolean> {
    clearTimeout(saveTimer);
    // A journal chunk mid-edit lives in the view's own state until something
    // finishes it — normally the blur from clicking elsewhere. A hotkey moves
    // no focus, so ask for it explicitly, while this is still the open note.
    journalView?.commitPending();
    // The debounced scroll-save may not have fired yet, and afterwards it would
    // credit this tab's position to the next one.
    clearTimeout(scrollSaveTimer);
    if (loadedPath) onscrollratio?.(scrollRatio(mode));
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
    // The note the buffer belongs to — the copy is its sibling.
    const v = loadedVault;
    const p = loadedPath;
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
      onopen?.(created, { focus: true });
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
    // The buffer's own note, not the pane's current props: a load in flight
    // has already cleared the buffer, so these always describe the same note
    // the text belongs to.
    const v = loadedVault;
    const p = loadedPath;
    // `base` = the text last synced to the backend; it merges base→c against
    // concurrent peer edits so a remote change isn't clobbered (#99).
    const base = lastLoaded;
    if (!v || !p || c === base) return;
    // Editing a preview tab pins it — the note is yours now, so the next click
    // in the sidebar opens beside it instead of replacing it (#169). Idempotent
    // once pinned, which is what keeps this out of its own dependencies.
    onedit?.();
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

</script>

<!-- The pane fills its share of the row; `relative` anchors the save banner
     over this note rather than the window. -->
<div class="relative flex min-w-0 flex-1 flex-col">
    <!-- A save that isn't happening, said out loud (#253). Fixed rather than in
       flow so it can't shift the editor's layout, and offset below the header
       so it clears the floating mobile chrome. -->
  {#if saveBlocked}
    <div
      class="absolute left-1/2 z-40 flex max-w-[min(30rem,calc(100%-2rem))] -translate-x-1/2 items-center gap-3 rounded-2xl border border-border bg-popover px-4 py-2 text-sm shadow-lg"
      style="top:calc(var(--chrome-h, 0px) + 0.5rem);"
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
    {#if view === "journal"}
      <JournalView bind:this={journalView} bind:value={content} {mobile} {kbOpen} {notePaths} {ontag} {oninternallink} />
    {:else if view === "preview"}
      <Preview
        bind:value={content}
        {notePaths}
        {oninternallink}
        {ontag}
        loadNote={(p) => readNote(vault, p)}
      />
    {:else}
      {#key openToken}
        <Editor
          bind:value={content}
          bind:view={editorView}
          bind:focused={editorFocused}
          {notePaths}
          focusOnMount={focusNew}
          {ontag}
        />
      {/key}
    {/if}
  </main>
</div>

<!-- Mobile: the markdown toolbar, anchored above the soft keyboard while this
     pane's editor has focus. -->
{#if mobile && mode === "source" && editorFocused && editorView && kbOpen}
  <MarkdownToolbar view={editorView} />
{/if}
