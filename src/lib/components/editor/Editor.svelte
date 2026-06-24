<script lang="ts">
  import { EditorView, keymap } from "@codemirror/view";
  import { EditorState } from "@codemirror/state";
  import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
  import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
  import { languages } from "@codemirror/language-data";
  import { onMount } from "svelte";
  import { thingsTheme } from "./things-theme";

  let {
    value = $bindable(""),
    selectOnMount = null,
  }: { value?: string; selectOnMount?: { from: number; to: number } | null } = $props();

  let container: HTMLDivElement;
  let view: EditorView | undefined;

  // Create the editor once. onMount is non-reactive, so reading props here
  // does not subscribe the editor to them — keystrokes won't recreate the view.
  onMount(() => {
    view = new EditorView({
      parent: container,
      state: EditorState.create({
        doc: value,
        // For a freshly created note, preselect the "Untitled" title so typing
        // immediately renames it (the filename follows the first H1).
        selection: selectOnMount
          ? { anchor: selectOnMount.from, head: selectOnMount.to }
          : undefined,
        extensions: [
          history(),
          keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
          markdown({ base: markdownLanguage, codeLanguages: languages }),
          EditorView.lineWrapping,
          thingsTheme,
          EditorView.updateListener.of((update) => {
            if (update.docChanged) {
              value = update.state.doc.toString();
            }
          }),
        ],
      }),
    });
    if (selectOnMount) view.focus();

    return () => {
      view?.destroy();
      view = undefined;
    };
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
