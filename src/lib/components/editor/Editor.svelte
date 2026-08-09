<script lang="ts">
  import { EditorView, keymap } from "@codemirror/view";
  import { EditorState, Compartment } from "@codemirror/state";
  import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
  import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
  import { languages } from "@codemirror/language-data";
  import {
    closeBrackets,
    closeBracketsKeymap,
    autocompletion,
    completionKeymap,
    type CompletionContext,
    type CompletionResult,
  } from "@codemirror/autocomplete";
  import { onMount } from "svelte";
  import { thingsTheme } from "./things-theme";
  import { theme } from "$lib/theme.svelte";
  import { editorSettings, contentAttrs } from "$lib/editor-settings.svelte";
  import { wrapInline, insertLink, toggleLinePrefix } from "./markdown-actions";
  import { frontmatterBadge } from "./frontmatter-badge";
  import { taskCheckboxes } from "./task-checkbox";
  import { blockBands } from "./block-bands";
  import { noteTypeOf, taskContinuation } from "$lib/note-type";

  // Desktop style hotkeys. Each toggles/applies markdown to the selection (the
  // same actions as the mobile toolbar). Bound ahead of the default keymap so
  // they take precedence. Mod = Cmd (macOS) / Ctrl (elsewhere).
  const styleKeymap = [
    { key: "Mod-b", run: (v: EditorView) => (wrapInline(v, "**"), true) },
    { key: "Mod-i", run: (v: EditorView) => (wrapInline(v, "*"), true) },
    { key: "Mod-e", run: (v: EditorView) => (wrapInline(v, "`"), true) },
    { key: "Mod-Shift-x", run: (v: EditorView) => (wrapInline(v, "~~"), true) },
    { key: "Mod-k", run: (v: EditorView) => (insertLink(v), true) },
    // Toggle a task-list checkbox on each selected line.
    { key: "Mod--", run: (v: EditorView) => (toggleLinePrefix(v, "- [ ] "), true) },
  ];

  // In a TODO note (#104), Enter at the end of a task line starts the next item,
  // the way a list behaves in any todo app. Gated on the note's own type so an
  // ordinary Markdown note's Enter is untouched, and it returns false on an
  // empty task line so the list can be ended by pressing Enter twice.
  const taskEnter = {
    key: "Enter",
    run: (v: EditorView) => {
      const state = v.state;
      if (noteTypeOf(state.doc.sliceString(0, Math.min(state.doc.length, 1024))) !== "todo")
        return false;
      const { from, to } = state.selection.main;
      if (from !== to) return false;
      const line = state.doc.lineAt(from);
      // Only at end-of-line, so Enter mid-line still just splits it.
      if (from !== line.to) return false;
      const cont = taskContinuation(line.text);
      if (!cont) return false;
      v.dispatch({
        changes: { from, insert: "\n" + cont },
        selection: { anchor: from + 1 + cont.length },
        scrollIntoView: true,
      });
      return true;
    },
  };

  // Theme lives in a compartment so light/dark (and OS) changes can reconfigure
  // it without recreating the editor.
  const themeConf = new Compartment();
  // Input-assist settings (close-brackets behavior + the content DOM's
  // autocomplete/autocapitalize/autocorrect/spellcheck attributes) each live in
  // a compartment so the settings sheet can toggle them live without a remount.
  const bracketsConf = new Compartment();
  const attrsConf = new Compartment();

  let {
    value = $bindable(""),
    view = $bindable<EditorView | undefined>(undefined),
    focused = $bindable(false),
    notePaths = [],
    focusOnMount = false,
  }: {
    value?: string;
    // Exposed so a sibling (the mobile markdown toolbar) can dispatch commands.
    view?: EditorView;
    focused?: boolean;
    // Vault note paths, for [[wiki link]] autocomplete.
    notePaths?: string[];
    // Focus the editor as soon as it mounts (a new note opened — #50). Done here
    // (not via an effect in the parent) so it targets this fresh instance at the
    // right moment, transferring the soft keyboard the name dialog held open.
    focusOnMount?: boolean;
  } = $props();

  let container: HTMLDivElement;

  // Cancels the scroll-pin from the previous tap (see pointerdown handler), so a
  // fresh gesture never fights a stale pin still re-asserting the old position.
  let cancelScrollPin: (() => void) | null = null;

  // Set the instant we start tearing the view down. Destroying CodeMirror fires a
  // synchronous `blur`, whose handler would mutate the bindable `focused` during
  // effect cleanup — Svelte 5 throws `state_unsafe_mutation`, and that error
  // aborts the same reactive flush that would place the quick-edit caret (#122).
  // Skip the focus/blur writes once teardown has begun.
  let tearingDown = false;

  // The completion source runs inside the editor (created once in onMount), so
  // it reads this mutable holder rather than the captured prop — kept current
  // by the effect below.
  let paths: string[] = [];
  $effect(() => {
    paths = notePaths;
  });

  // Autocomplete note paths while typing inside `[[ … ]]`. Offers each note's
  // path; accepting inserts it and the closing `]]` (reusing any the
  // close-brackets pair already added) and drops the caret after them. Pairs
  // with the preview-side resolver in Preview.svelte (#16/#43).
  function wikiLinkCompletions(context: CompletionContext): CompletionResult | null {
    const token = context.matchBefore(/\[\[[^\]\n]*/);
    if (!token) return null;
    if (token.from + 2 === context.pos && !context.explicit) {
      // Just typed the second `[`; wait for a character (or explicit trigger).
    }
    const from = token.from + 2; // after the `[[`
    return {
      from,
      validFor: /^[^\]\n]*$/,
      options: paths.map((p) => ({
        label: p,
        type: "text",
        apply: (v: EditorView, _c: unknown, a: number, b: number) => {
          const hasClose = v.state.sliceDoc(b, b + 2) === "]]";
          const insert = p + (hasClose ? "" : "]]");
          v.dispatch({
            changes: { from: a, to: b, insert },
            selection: { anchor: a + p.length + 2 },
          });
        },
      })),
    };
  }

  // Create the editor once. onMount is non-reactive, so reading props here
  // does not subscribe the editor to them — keystrokes won't recreate the view.
  onMount(() => {
    view = new EditorView({
      parent: container,
      state: EditorState.create({
        doc: value,
        extensions: [
          history(),
          frontmatterBadge,
          taskCheckboxes,
          blockBands,
          keymap.of([
            taskEnter,
            ...styleKeymap,
            ...closeBracketsKeymap,
            ...completionKeymap,
            ...defaultKeymap,
            ...historyKeymap,
            indentWithTab,
          ]),
          autocompletion({ override: [wikiLinkCompletions], icons: false }),
          markdown({ base: markdownLanguage, codeLanguages: languages }),
          EditorView.lineWrapping,
          // NB: no keyboard-inset scrollMargin here. The editor container is
          // shrunk by --editor-kb-inset (Editor's wrapper) so the scroller sits
          // entirely above the keyboard; a bottom scrollMargin on top of that
          // would over-scroll scroll-into-view (e.g. tapping a line yanked the
          // viewport toward the top — #66).
          themeConf.of(thingsTheme(theme.dark)),
          bracketsConf.of(editorSettings.closeBrackets ? closeBrackets() : []),
          attrsConf.of(EditorView.contentAttributes.of(contentAttrs())),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) {
              value = update.state.doc.toString();
            }
          }),
          // Track focus so the mobile toolbar only shows while editing.
          EditorView.domEventHandlers({
            // Preserve scroll across a tap. The scroller sits entirely above the
            // keyboard (the wrapper is shrunk by --editor-kb-inset), so tapping a
            // visible line never needs a scroll — yet a tap could still yank the
            // viewport away: the selection/keyboard re-layout scrolls the *prior*
            // caret into view (to the top — #66; or back to the old caret after
            // you'd scrolled away — #101).
            //
            // A one-shot snap-back guesses when that scroll fires and loses the
            // race when it lands late (#101). Instead pin scrollTop for the whole
            // gesture + a short settle, re-asserting on every scroll event, until
            // the user actually drags (then it's a real scroll/selection — let go)
            // or the next gesture starts.
            pointerdown: (e, v) => {
              cancelScrollPin?.();
              const top = v.scrollDOM.scrollTop;
              const startY = e.clientY;
              const onScroll = () => {
                if (v.scrollDOM.scrollTop !== top) v.scrollDOM.scrollTop = top;
              };
              const onMove = (ev: PointerEvent) => {
                if (Math.abs(ev.clientY - startY) > 8) release();
              };
              let timer = 0;
              const release = () => {
                cancelScrollPin = null;
                clearTimeout(timer);
                v.scrollDOM.removeEventListener("scroll", onScroll);
                window.removeEventListener("pointermove", onMove);
                window.removeEventListener("touchmove", release);
                window.removeEventListener("pointercancel", release);
                window.removeEventListener("pointerup", onUp);
              };
              // Keep pinning briefly past the release to absorb a late layout
              // scroll, then stop.
              const onUp = () => (timer = window.setTimeout(release, 250));
              cancelScrollPin = release;
              v.scrollDOM.addEventListener("scroll", onScroll);
              window.addEventListener("pointermove", onMove);
              // A touch-scroll fires touchmove (and WebKit a pointercancel as it
              // takes over) before the scroll lands — release then so we never
              // fight a genuine user scroll.
              window.addEventListener("touchmove", release, { passive: true });
              window.addEventListener("pointercancel", release);
              window.addEventListener("pointerup", onUp);
              return false;
            },
            focus: () => {
              if (!tearingDown) focused = true;
              return false;
            },
            blur: () => {
              if (!tearingDown) focused = false;
              return false;
            },
          }),
        ],
      }),
    });

    // New note: focus now, on this fresh instance — the keyboard is already up
    // (held by the name dialog's keeper) and transfers here.
    //
    // Put the caret at the END of the document, not CodeMirror's default of
    // position 0. A typed note is seeded with a starter body (a TODO note gets
    // `- [ ] `), and starting at 0 put the caret BEFORE that marker — so typing
    // pushed the marker along ahead of the text and produced `Milk- [ ] `
    // instead of `- [ ] Milk`, with the checkbox disappearing because the marker
    // was no longer at the line start. Harmless for a plain note, whose document
    // is empty, so end == 0 anyway.
    if (focusOnMount) {
      view.dispatch({ selection: { anchor: view.state.doc.length } });
      view.focus();
    }

    return () => {
      tearingDown = true;
      cancelScrollPin?.();
      view?.destroy();
      view = undefined;
    };
  });

  // Reconfigure the editor theme when light/dark (or the OS preference) flips.
  $effect(() => {
    const dark = theme.dark;
    view?.dispatch({ effects: themeConf.reconfigure(thingsTheme(dark)) });
  });

  // Reconfigure input-assist when the settings sheet toggles them. Reading each
  // getter subscribes this effect to its changes.
  $effect(() => {
    const brackets = editorSettings.closeBrackets;
    view?.dispatch({
      effects: bracketsConf.reconfigure(brackets ? closeBrackets() : []),
    });
  });
  $effect(() => {
    // Touch each so the effect re-runs when any attribute setting changes.
    editorSettings.autocomplete;
    editorSettings.autocapitalize;
    editorSettings.autocorrect;
    editorSettings.spellcheck;
    view?.dispatch({
      effects: attrsConf.reconfigure(EditorView.contentAttributes.of(contentAttrs())),
    });
  });

  // Push external `value` changes into the editor. Guarded against the echo
  // from our own updateListener so it never dispatches mid-typing.
  //
  // An external change here is an incoming P2P sync update to the open note, so
  // apply it as a *minimal* edit — keep the common prefix/suffix and replace
  // only the differing middle. CodeMirror then maps the caret, selection, and
  // scroll position through the change instead of resetting them, so a peer
  // who is actively editing isn't disrupted (issue #25). Replacing the whole
  // doc (from 0 to length) collapsed the caret to the top and reset scroll.
  $effect(() => {
    const next = value ?? "";
    if (!view) return;
    const cur = view.state.doc.toString();
    if (next === cur) return;

    let start = 0;
    const max = Math.min(cur.length, next.length);
    while (start < max && cur[start] === next[start]) start++;
    let endCur = cur.length;
    let endNext = next.length;
    while (endCur > start && endNext > start && cur[endCur - 1] === next[endNext - 1]) {
      endCur--;
      endNext--;
    }

    view.dispatch({
      changes: { from: start, to: endCur, insert: next.slice(start, endNext) },
      // A remote edit shouldn't yank the viewport to the changed region.
      scrollIntoView: false,
    });
  });
</script>

<!-- The bottom inset (soft keyboard + mobile toolbar; 0 otherwise) shrinks the
     editor box so the scroller sits *above* the keyboard, instead of padding the
     content (which created a tall empty region you could scroll/tap into, #66). -->
<div
  bind:this={container}
  class="h-full w-full overflow-hidden"
  style="padding-bottom: var(--editor-kb-inset, 0px);"
></div>
