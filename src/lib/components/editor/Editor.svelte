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
          keymap.of([
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
          // Treat the keyboard+toolbar occlusion (--editor-kb-inset; 0 when no
          // keyboard) as invisible bottom space, so CM's own scroll-into-view
          // on each keystroke keeps the caret clear of it.
          EditorView.scrollMargins.of((v) => {
            const h = parseInt(getComputedStyle(v.dom).getPropertyValue("--editor-kb-inset"));
            return h ? { bottom: h } : null;
          }),
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
            focus: () => {
              focused = true;
              return false;
            },
            blur: () => {
              focused = false;
              return false;
            },
          }),
        ],
      }),
    });

    // New note: focus now, on this fresh instance — the keyboard is already up
    // (held by the name dialog's keeper) and transfers here.
    if (focusOnMount) view.focus();

    return () => {
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
