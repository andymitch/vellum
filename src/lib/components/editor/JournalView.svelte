<script lang="ts">
  // Journal is a long-running note divided into separately editable chunks
  // (#227) — a notebook of cells, not a chat log or a file per day. A chunk
  // renders as read-only markdown; clicking one drops it into an editable
  // field, and finishing commits the change back.
  //
  // A placeholder chunk always sits at the end of the note, and writing in it
  // is how a new chunk is made — finish it and the next placeholder appears
  // below. Deliberately not a keyboard shortcut: a shortcut needs a modifier
  // to chord with (a bare Enter can't be told apart from a shift-modified one
  // on a mobile keyboard), and a modifier is exactly what a touch keyboard
  // doesn't have. A placeholder works the same everywhere.
  //
  // Finishing a chunk is Return, and Shift+Return inserts a line break;
  // editorSettings.journalReturnNewline swaps the two. Neither applies on
  // mobile, where Enter is always a line break and finishing means dismissing
  // the keyboard (which blurs the field).
  //
  // Chunks can be dragged to reorder (desktop only). Each tracks a created
  // time and, once actually edited, an updated time, shown on a timeline in
  // the right margin — outside the chunk, so it can never render over the
  // chunk's own content.
  //
  // Storage is still a plain .md file. Each chunk's created/updated times are
  // an HTML comment marker, invisible wherever the note is rendered, and
  // content with no marker is kept rather than discarded — same principle as
  // a TodoRow that isn't a task (note-type.ts).
  import { tick } from "svelte";
  import { isToday, isYesterday, format } from "date-fns";
  import {
    parseJournalCells,
    serializeJournalCells,
    type JournalCell,
  } from "$lib/note-type";
  import { editorSettings } from "$lib/editor-settings.svelte";
  import { renderMarkdown, resolveWikiLink } from "$lib/render-markdown";

  let {
    value = $bindable(""),
    notePaths = [],
    mobile = false,
    oninternallink,
    ontag,
  }: {
    value?: string;
    notePaths?: string[];
    mobile?: boolean;
    oninternallink?: (path: string, fragment?: string) => void;
    ontag?: (tag: string) => void;
  } = $props();

  const cells = $derived(parseJournalCells(value));
  // One row past the real chunks is the placeholder: the way a new chunk gets
  // made, and on a brand-new note the only row there is. It becomes a real,
  // timestamped chunk once something is typed into it (see commitEdit), at
  // which point the next placeholder takes its place at the end.
  const displayCells = $derived([
    ...cells,
    { created: null, updated: null, text: "" } as JournalCell,
  ]);

  function commit(next: JournalCell[]) {
    value = serializeJournalCells(value, next);
  }

  // A compact hover timestamp: relative while it happened today ("3m ago"),
  // otherwise a bare clock time ("at 4:02 PM") — today's own date being
  // obvious from context. Note this drops the date entirely once a chunk is
  // more than a day old, which is a real ambiguity for anything from, say,
  // last month; worth revisiting if that turns out to matter in practice.
  function relativeOrAbsolute(iso: string): string {
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return "";
    if (isToday(d)) {
      const minutes = Math.floor((Date.now() - d.getTime()) / 60_000);
      if (minutes < 1) return "just now";
      if (minutes < 60) return `${minutes}m ago`;
      return `${Math.floor(minutes / 60)}h ago`;
    }
    return `at ${format(d, "h:mm a")}`;
  }

  const cellDates = $derived(
    cells.map((c) => {
      if (!c.created) return null;
      const d = new Date(c.created);
      return Number.isNaN(d.getTime()) ? null : d;
    }),
  );

  function dayLabel(d: Date): string {
    if (isToday(d)) return "Today";
    if (isYesterday(d)) return "Yesterday";
    const sameYear = d.getFullYear() === new Date().getFullYear();
    return sameYear ? format(d, "MMM d") : format(d, "MMM d, yyyy");
  }

  // What the rail shows at rest: a day label against the first chunk written
  // on each day, so the margin reads as a chronology rather than a column of
  // identical dots. Hovering swaps it for that chunk's exact times.
  const dayMarkers = $derived.by(() => {
    let prev: Date | null = null;
    return cellDates.map((d) => {
      if (!d) return null;
      const changed = !prev || d.toDateString() !== prev.toDateString();
      prev = d;
      return changed ? dayLabel(d) : null;
    });
  });

  // Grows a <textarea> to fit its content instead of scrolling internally.
  function autoGrow(el: HTMLTextAreaElement) {
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }

  // Tag/wikilink clicks inside a rendered (read-only) chunk. Wikilink
  // resolution happens lazily here at click time rather than pre-computed per
  // chunk (as Preview.svelte does for the whole note): chunks are short, and
  // this avoids a per-chunk DOM effect just to pre-style broken links. The
  // trade-off is a wikilink in a chunk always reads as a normal link rather
  // than showing broken-link styling up front.
  //
  // Returns whether the click was handled here, so the caller (entering edit
  // mode) knows to skip that — clicking a tag/link should navigate, not also
  // drop the chunk you clicked out of into an editable field underneath it.
  function onCellClick(e: MouseEvent): boolean {
    const a = (e.target as HTMLElement | null)?.closest("a");
    if (!a) return false;
    e.preventDefault();
    if (a.classList.contains("tagchip")) {
      const tag = a.dataset.tag;
      if (tag) ontag?.(tag);
      return true;
    }
    if (a.classList.contains("wikilink")) {
      const target = a.dataset.target ?? "";
      const path = target ? resolveWikiLink(target, notePaths) : null;
      if (path) oninternallink?.(path);
      return true;
    }
    return false;
  }

  // ---- Editing a chunk ----
  let editingIndex = $state<number | null>(null);
  let editText = $state("");
  let editEl = $state<HTMLTextAreaElement | undefined>(undefined);
  // Splitting and deleting rewrite the chunk list themselves and hand focus to
  // a different chunk. That tears down the <textarea> being edited, which can
  // fire its own blur handler — and a second commit over the state we just
  // wrote would undo it (deleting, for instance, the empty chunk a split had
  // only just created). Set while that handoff is in flight.
  let reconciling = false;

  async function focusEditor(cursor: "start" | "end") {
    await tick();
    const el = editEl;
    if (el) {
      el.focus();
      const pos = cursor === "start" ? 0 : el.value.length;
      el.setSelectionRange(pos, pos);
      autoGrow(el);
      el.scrollIntoView({ block: "nearest" });
    }
    reconciling = false;
  }

  function startEdit(i: number, cursor: "start" | "end" = "end") {
    editingIndex = i;
    editText = cells[i]?.text ?? "";
    void focusEditor(cursor);
  }

  function commitEdit() {
    if (reconciling || editingIndex === null) return;
    const i = editingIndex;
    const text = editText.trim();
    editingIndex = null;
    const original = cells[i];
    const next = [...cells];
    if (!text) {
      // An untouched placeholder: nothing to remove and nothing to write, so
      // don't re-serialize the note just because it was clicked into.
      if (!original) return;
      next.splice(i, 1);
      commit(next);
      return;
    }
    const now = new Date().toISOString();
    if (original) {
      next[i] = { ...original, text, updated: text !== original.text ? now : original.updated };
    } else {
      // No chunk at this index: the note was empty and this is its first one.
      next.push({ created: now, updated: null, text });
    }
    commit(next);
  }

  // Backspacing an already-empty chunk deletes it and merges focus into the
  // end of the previous one, so an empty chunk never lingers once you've
  // backed all the way out of it.
  function removeCurrentCell() {
    if (editingIndex === null) return;
    const i = editingIndex;
    reconciling = true;
    editingIndex = null;
    commit(cells.filter((_, idx) => idx !== i));
    // Nothing before the first chunk to merge into, so that one just closes.
    if (i > 0) startEdit(i - 1, "end");
    else void focusEditor("end");
  }

  function onEditKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      editEl?.blur();
      return;
    }
    // Blurring is what commits (see commitEdit), so finishing is just a blur.
    // Mobile is left alone entirely: Enter inserts a line break there, and
    // dismissing the keyboard is what finishes the chunk.
    if (e.key === "Enter" && !mobile) {
      const finishes = editorSettings.journalReturnNewline ? e.shiftKey : !e.shiftKey;
      if (finishes) {
        e.preventDefault();
        editEl?.blur();
      }
      return;
    }
    if (e.key === "Backspace" && editText === "") {
      e.preventDefault();
      removeCurrentCell();
    }
  }

  // ---- Reordering by drag (desktop only) ----
  // dropIndex is a position *between* chunks (0 … cells.length), not a chunk
  // index — it's where the dragged chunk would land, and it's what the drop
  // line draws itself at.
  let dragIndex = $state<number | null>(null);
  let dropIndex = $state<number | null>(null);
  // Kept as explicit state rather than inferred through a sibling :hover
  // selector: the following rail needs to know that its preceding cell is
  // active, and this remains reliable as the pointer moves between a cell,
  // its gutter and its timeline.
  let hoveredIndex = $state<number | null>(null);

  function onDragStart(e: DragEvent, i: number) {
    dragIndex = i;
    e.dataTransfer?.setData("text/plain", String(i));
    if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
  }
  function onDragOver(e: DragEvent, i: number) {
    if (dragIndex === null) return;
    e.preventDefault();
    // Past the midpoint of a chunk means "after it", so the line can land in
    // either gap around whichever chunk the pointer happens to be over.
    const box = (e.currentTarget as HTMLElement).getBoundingClientRect();
    // Clamped because the last row is the placeholder, not a chunk: the
    // furthest a chunk can land is after the final real one.
    dropIndex = Math.min(e.clientY > box.top + box.height / 2 ? i + 1 : i, cells.length);
  }
  function onDrop(e: DragEvent) {
    e.preventDefault();
    const from = dragIndex;
    const to = dropIndex;
    dragIndex = null;
    dropIndex = null;
    if (from === null || to === null) return;
    // Both gaps touching the dragged chunk put it back where it started.
    if (to === from || to === from + 1) return;
    const next = [...cells];
    const [moved] = next.splice(from, 1);
    next.splice(to > from ? to - 1 : to, 0, moved);
    commit(next);
  }
  function onDragEnd() {
    dragIndex = null;
    dropIndex = null;
  }
</script>

<div class="journal-scroll h-full min-h-0 overflow-y-auto px-4 py-4" role="list">
  {#each displayCells as cell, i (i)}
    <!-- Three columns: the chunk keeps a uniform full-width column of its own,
         and the timeline gets the right margin beside it — never a slice of
         the chunk's own width. The whole band is the drop target, so a drag
         doesn't have to stay inside the chunk to aim between two of them. -->
    <div
      class="cell-row-wrap"
      role="listitem"
      ondragover={(e) => onDragOver(e, i)}
      ondrop={onDrop}
      onmouseenter={() => (hoveredIndex = i)}
      onmouseleave={() => (hoveredIndex = null)}
    >
      <div class="cell-slot">
        {#if dropIndex === i}
          <div class="drop-line" aria-hidden="true"></div>
        {/if}
        {#if editingIndex === i}
          <textarea
            bind:this={editEl}
            bind:value={editText}
            class="cell-editor"
            rows="1"
            placeholder="Write something…"
            autocapitalize="sentences"
            spellcheck="true"
            oninput={(e) => autoGrow(e.currentTarget)}
            onkeydown={onEditKeydown}
            onblur={commitEdit}
            {...{ autocorrect: "on" }}
          ></textarea>
        {:else if i === cells.length}
          <button type="button" class="cell-placeholder" onclick={() => startEdit(i)}>
            New note…
          </button>
        {:else}
          <div
            class="cell-row"
            class:dragging={dragIndex === i}
            role="button"
            tabindex="0"
            draggable={!mobile}
            ondragstart={(e) => onDragStart(e, i)}
            ondragend={onDragEnd}
            onclick={(e) => {
              if (!onCellClick(e)) startEdit(i);
            }}
            onkeydown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                startEdit(i);
              }
            }}
          >
            <!-- eslint-disable-next-line svelte/no-at-html-tags -->
            <div class="prose-content">{@html renderMarkdown(cell.text)}</div>
          </div>
        {/if}
      </div>

      {#if !mobile && i < cells.length}
        <div
          class="cell-rail"
          class:has-next={i < cells.length - 1}
          aria-hidden="true"
        >
          <!-- The blue top-half of a connection whose preceding chunk is
               hovered. Kept separate from the faint rail so that rail still
               continues below this dot. -->
          <span
            class="rail-incoming"
            style:background={hoveredIndex === i - 1
              ? "color-mix(in srgb, var(--editor-accent) 55%, transparent)"
              : "transparent"}
          ></span>
          <span class="rail-dot" class:untimed={!cell.created}></span>
          {#if dayMarkers[i]}
            <span class="rail-day">{dayMarkers[i]}</span>
          {/if}
          {#if cell.created}
            <div class="rail-times">
              <span>created {relativeOrAbsolute(cell.created)}</span>
              {#if cell.updated && cell.updated !== cell.created}
                <span>updated {relativeOrAbsolute(cell.updated)}</span>
              {/if}
            </div>
          {/if}
        </div>
      {/if}
    </div>
  {/each}
</div>

<style>
  /* Sized against the pane, not the window, so the rail hides when the
     sidebar squeezes the note rather than when the window is small. */
  .journal-scroll {
    container-type: inline-size;
  }

  /* A centred chunk column with a margin either side; the timeline lives in
     the right one. Equal 1fr margins keep the chunk column centred, and the
     chunk itself always fills it — its width never changes with hover, or
     with whether a chunk happens to carry timestamps. */
  .cell-row-wrap {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 48rem) minmax(0, 1fr);
  }
  .cell-slot {
    position: relative;
    grid-column: 2;
    padding-bottom: 0.5rem;
  }

  .cell-row {
    position: relative;
    margin: 0 -0.5rem;
    padding: 0.15rem 0.5rem;
    border-radius: 0.375rem;
    cursor: pointer;
  }
  .cell-row:hover {
    background: color-mix(in srgb, var(--editor-fg) 3%, transparent);
  }
  .cell-row.dragging {
    opacity: 0.4;
  }
  .cell-row :global(.prose-content) {
    line-height: 1.5;
  }
  /* First/last block inside a chunk shouldn't carry the extra top/bottom
     margin .prose-content's own rules give a full note preview. */
  .cell-row :global(.prose-content > :first-child) {
    margin-top: 0;
  }
  .cell-row :global(.prose-content > :last-child) {
    margin-bottom: 0;
  }

  /* Where a dragged chunk would land: a line of its own, drawn in the gap
     between two chunks rather than as an edge on either of them. Absolute, so
     showing it never nudges the chunks it sits between. */
  .drop-line {
    position: absolute;
    left: -0.5rem;
    right: -0.5rem;
    top: -0.3rem;
    height: 2px;
    border-radius: 1px;
    background: var(--editor-accent);
    pointer-events: none;
  }
  .drop-line-end {
    top: auto;
    bottom: 0.2rem;
  }

  /* ---- The timeline, in the right margin ----
     A continuous rail (each row draws its own segment, so they join up), a dot
     per chunk, and a label that at rest marks the first chunk of each day and
     on hover becomes that chunk's exact times. Everything here is absolutely
     positioned: the rail must never influence the height of the row, or a
     two-line timestamp would push short chunks apart. */
  .cell-rail {
    position: relative;
    grid-column: 3;
    user-select: none;
  }
  .cell-rail::before,
  .cell-rail::after {
    content: "";
    position: absolute;
    /* A 1px rail is positioned by its left edge. Nudge that edge half a pixel
       left so the rail's *centre* shares the dot's centre coordinate. */
    left: calc(1.5rem - 0.5px);
    width: 1px;
  }
  /* The quiet rail: spans the whole row, so consecutive rows' segments meet
     and read as one continuous line. */
  .cell-rail::before {
    top: 0;
    bottom: 0;
    background: color-mix(in srgb, var(--editor-border) 70%, transparent);
  }
  /* The first half of a highlighted connection: from this dot to the end of
     its row. This deliberately stays inside the rail's row — the scroll pane
     clips an overflowing pseudo-element at the row boundary. The matching
     second half, in the next row's ::before, fills from that boundary down to
     the following dot. Together they are a genuinely continuous dot-to-dot
     line without relying on an overflowing endpoint. */
  .cell-rail::after {
    top: calc(0.15rem + 0.75em);
    bottom: 0;
    background: transparent;
    transition: background-color 140ms ease;
  }
  .cell-row-wrap:hover .cell-rail.has-next::after {
    background: color-mix(in srgb, var(--editor-accent) 55%, transparent);
  }
  /* The second half of the connection. It lives independently of ::before,
     which must remain the complete faint rail below this dot as well. */
  .rail-incoming {
    position: absolute;
    left: calc(1.5rem - 0.5px);
    top: 0;
    width: 1px;
    height: calc(0.15rem + 0.75em);
    background: transparent;
    transition: background-color 140ms ease;
  }
  /* Centred on the chunk's first line: its own top padding, plus half a line. */
  .rail-dot {
    position: absolute;
    left: 1.5rem;
    top: calc(0.15rem + 0.75em);
    width: 7px;
    height: 7px;
    border-radius: 9999px;
    /* Blend with the background, not transparency: a translucent dot lets
       the rail show through it, making a connection look like it continues
       inside the marker rather than ending cleanly at its edge. */
    background: color-mix(in srgb, var(--editor-muted) 55%, var(--background));
    box-shadow: 0 0 0 2px var(--background);
    z-index: 1;
    transform: translate(-50%, -50%);
    transition:
      transform 140ms ease,
      background-color 140ms ease;
  }
  /* A chunk with no marker (legacy, or written by hand elsewhere) still gets a
     dot so the rhythm of the rail matches the note, but an outline one: there
     is no time to show for it. */
  .rail-dot.untimed {
    /* The centre is still an opaque patch of the page. A transparent ring
       would let the timeline behind it show through, so it reads as a line
       drawn through a dot rather than a deliberately unfilled marker. */
    background: var(--background);
    box-shadow:
      0 0 0 2px var(--background),
      inset 0 0 0 1px color-mix(in srgb, var(--editor-muted) 55%, transparent);
  }
  .cell-row-wrap:hover .rail-dot {
    background: var(--editor-accent);
    box-shadow: 0 0 0 2px var(--background);
    transform: translate(-50%, -50%) scale(1.5);
  }

  /* Day label and exact times occupy the same spot and cross-fade, so the
     margin stays a single quiet column either way. */
  .rail-day,
  .rail-times {
    position: absolute;
    left: 2.4rem;
    top: 0.1rem;
    font-size: 0.7rem;
    line-height: 1.4;
    white-space: nowrap;
    transition:
      opacity 140ms ease,
      transform 140ms ease;
  }
  .rail-day {
    font-size: 0.65rem;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--editor-muted) 65%, transparent);
    opacity: 1;
  }
  .rail-times {
    opacity: 0;
    transform: translateX(-0.25rem);
    color: color-mix(in srgb, var(--editor-muted) 85%, transparent);
  }
  .rail-times span {
    display: block;
  }
  .cell-row-wrap:hover .rail-day {
    opacity: 0;
  }
  .cell-row-wrap:hover .rail-times {
    opacity: 1;
    transform: translateX(0);
  }
  /* A two-line timestamp reaches into the row below it, so that row's day
     label steps aside rather than colliding with it. */
  .cell-row-wrap:hover + .cell-row-wrap .rail-day {
    opacity: 0;
  }

  /* No margin worth speaking of: drop the rail rather than crush the note. */
  @container (max-width: 62rem) {
    .cell-rail {
      display: none;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .cell-rail::before,
    .rail-dot,
    .rail-incoming,
    .rail-day,
    .rail-times {
      transition: none;
    }
  }

  .cell-editor {
    width: calc(100% + 1rem);
    resize: none;
    overflow: hidden;
    margin: 0 -0.5rem;
    border-radius: 0.5rem;
    padding: 0.15rem 0.5rem;
    line-height: 1.5;
    font-family: var(--font-sans);
    color: var(--editor-fg);
    background: color-mix(in srgb, var(--editor-fg) 4%, transparent);
    border: 1px solid var(--editor-border);
    outline: none;
  }
  .cell-editor:focus {
    border-color: var(--editor-accent);
  }

  /* Reads as a chunk that hasn't been written yet: same geometry as a real
     one, dimmed, waiting at the end of the note. */
  .cell-placeholder {
    display: block;
    width: calc(100% + 1rem);
    margin: 0 -0.5rem;
    padding: 0.15rem 0.5rem;
    border-radius: 0.375rem;
    text-align: left;
    line-height: 1.5;
    color: color-mix(in srgb, var(--editor-muted) 70%, transparent);
    cursor: text;
  }
  .cell-placeholder:hover {
    background: color-mix(in srgb, var(--editor-fg) 3%, transparent);
    color: var(--editor-muted);
  }
</style>
