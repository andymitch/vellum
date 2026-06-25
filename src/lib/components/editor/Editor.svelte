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
  $effect(() => {
    if (view && value !== view.state.doc.toString()) {
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: value },
      });
    }
  });
</script>

<div bind:this={container} class="h-full w-full overflow-hidden"></div>
