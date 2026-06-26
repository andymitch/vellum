<script lang="ts">
  import { EditorView, keymap } from "@codemirror/view";
  import { EditorState, Compartment } from "@codemirror/state";
  import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
  import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
  import { languages } from "@codemirror/language-data";
  import { closeBrackets, closeBracketsKeymap } from "@codemirror/autocomplete";
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
  }: {
    value?: string;
    // Exposed so a sibling (the mobile markdown toolbar) can dispatch commands.
    view?: EditorView;
    focused?: boolean;
  } = $props();

  let container: HTMLDivElement;

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
            ...defaultKeymap,
            ...historyKeymap,
            indentWithTab,
          ]),
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

<div bind:this={container} class="h-full w-full overflow-hidden"></div>
