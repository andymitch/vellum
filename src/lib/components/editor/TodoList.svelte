<script lang="ts">
  // A TODO note as an actual checklist (#180) — think grocery list, not editor.
  //
  // What shipped first was a Markdown editor with checkbox widgets drawn over
  // it: visible bullets, raw text, caret semantics. Correct storage, wrong
  // surface. This is the surface: rows with a checkbox and a text field, drag to
  // reorder, a line through what's done.
  //
  // Storage is unchanged — `- [ ] milk` in the note body — so a TODO note is
  // still a plain .md file that syncs through the same CRDT and exports intact.
  // Every edit goes back through `value`, so it autosaves and merges like any
  // other change.
  import { GripVertical, Plus, X } from "@lucide/svelte";
  import { parseTodoRows, serializeTodoRows, type TodoRow } from "$lib/note-type";

  let { value = $bindable("") }: { value?: string } = $props();

  // Derived from the note, so a change arriving from another device (via the
  // vault-changed rebase) shows up here without any extra plumbing.
  const rows = $derived(parseTodoRows(value));

  function commit(next: TodoRow[]) {
    value = serializeTodoRows(value, next);
  }

  const replace = (i: number, patch: Partial<TodoRow>) =>
    commit(rows.map((r, n) => (n === i ? { ...r, ...patch } : r)));

  const remove = (i: number) => commit(rows.filter((_, n) => n !== i));

  function addRow() {
    commit([...rows, { task: true, checked: false, text: "" }]);
    // Focus the row we just added, once it exists.
    queueMicrotask(() => {
      const inputs = list?.querySelectorAll<HTMLInputElement>("input[type='text']");
      inputs?.[inputs.length - 1]?.focus();
    });
  }

  // Enter adds the next item, the way a list should behave; Backspace on an
  // empty row deletes it and steps back, so a stray row is easy to undo.
  function onKey(e: KeyboardEvent, i: number) {
    if (e.key === "Enter") {
      e.preventDefault();
      addRow();
    } else if (e.key === "Backspace" && rows[i].text === "" && rows.length > 1) {
      e.preventDefault();
      remove(i);
      queueMicrotask(() => {
        const inputs = list?.querySelectorAll<HTMLInputElement>("input[type='text']");
        inputs?.[Math.max(0, i - 1)]?.focus();
      });
    }
  }

  // ---- drag to reorder ----
  //
  // Pointer events rather than HTML5 drag-and-drop: this is primarily a phone
  // feature, and HTML5 dragging doesn't work on touch at all. Dragging is
  // started from the handle only, so a swipe over the list still scrolls.
  let list: HTMLDivElement;
  let dragFrom = $state(-1);
  let dragTo = $state(-1);

  function startDrag(e: PointerEvent, i: number) {
    e.preventDefault();
    dragFrom = i;
    dragTo = i;
    (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
  }

  function moveDrag(e: PointerEvent) {
    if (dragFrom < 0 || !list) return;
    // Which row is under the pointer, by row midpoints — steadier than
    // hit-testing the element under the finger, which the dragged row covers.
    const items = [...list.querySelectorAll<HTMLElement>("[data-row]")];
    let target = items.length - 1;
    for (let n = 0; n < items.length; n++) {
      const r = items[n].getBoundingClientRect();
      if (e.clientY < r.top + r.height / 2) {
        target = n;
        break;
      }
    }
    dragTo = target;
  }

  function endDrag() {
    if (dragFrom >= 0 && dragTo >= 0 && dragFrom !== dragTo) {
      const next = [...rows];
      const [moved] = next.splice(dragFrom, 1);
      next.splice(dragTo, 0, moved);
      commit(next);
    }
    dragFrom = -1;
    dragTo = -1;
  }
</script>

<div class="mx-auto w-full max-w-3xl px-4 py-4" bind:this={list}>
  {#each rows as row, i (i)}
    <div
      data-row
      class="group flex items-center gap-1 rounded-md py-0.5 transition-colors {dragFrom === i
        ? 'opacity-40'
        : ''} {dragTo === i && dragFrom >= 0 && dragFrom !== i
        ? 'ring-2 ring-primary/40'
        : ''}"
    >
      <button
        type="button"
        class="shrink-0 cursor-grab touch-none p-1 text-muted-foreground/40 hover:text-muted-foreground active:cursor-grabbing"
        aria-label="Reorder"
        onpointerdown={(e) => startDrag(e, i)}
        onpointermove={moveDrag}
        onpointerup={endDrag}
        onpointercancel={endDrag}
      >
        <GripVertical class="h-4 w-4" />
      </button>

      {#if row.task}
        <input
          type="checkbox"
          class="h-4 w-4 shrink-0 accent-primary"
          checked={row.checked}
          onchange={(e) => replace(i, { checked: e.currentTarget.checked })}
        />
        <input
          type="text"
          class="min-w-0 flex-1 bg-transparent px-1 py-1 text-sm outline-none {row.checked
            ? 'text-muted-foreground line-through'
            : ''}"
          value={row.text}
          placeholder="Item"
          autocapitalize="sentences"
          autocorrect="on"
          spellcheck="true"
          oninput={(e) => replace(i, { text: e.currentTarget.value })}
          onkeydown={(e) => onKey(e, i)}
        />
      {:else}
        <!-- Not a task line (prose, or something a peer wrote). Kept and
             editable rather than discarded, so nothing is lost. -->
        <input
          type="text"
          class="min-w-0 flex-1 bg-transparent px-1 py-1 text-sm text-muted-foreground outline-none"
          value={row.text}
          oninput={(e) => replace(i, { text: e.currentTarget.value })}
        />
      {/if}

      <button
        type="button"
        class="shrink-0 p-1 text-muted-foreground/0 transition-colors hover:text-destructive group-hover:text-muted-foreground/60"
        aria-label="Delete item"
        onclick={() => remove(i)}
      >
        <X class="h-4 w-4" />
      </button>
    </div>
  {/each}

  <button
    type="button"
    class="mt-1 flex w-full items-center gap-2 rounded-md px-2 py-2 text-sm text-muted-foreground hover:bg-muted"
    onclick={addRow}
  >
    <Plus class="h-4 w-4" />
    Add item
  </button>
</div>
