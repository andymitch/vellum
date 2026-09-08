<script lang="ts">
  import { X, Columns2, Ungroup } from "@lucide/svelte";
  import { isMacApp } from "$lib/platform";
  import { isJoined, tabLabels, type Tab } from "$lib/tabs";

  let {
    tabs,
    active,
    onselect,
    onclose,
    ondblclick,
    onreorder,
    onjoin,
    onsplit,
  }: {
    tabs: Tab[];
    active: number;
    onselect: (i: number) => void;
    onclose: (i: number) => void;
    // Promotes the tab: pins a preview tab (the same gesture that pins one from
    // the sidebar), renames an already-pinned one.
    ondblclick: (i: number) => void;
    // A tab was dragged along the strip and dropped at `to` (#266).
    onreorder: (from: number, to: number) => void;
    // A tab was dropped *onto* another one: show both notes side by side (#169).
    onjoin: (from: number, to: number) => void;
    // The split-apart button on a joined tab.
    onsplit: (i: number) => void;
  } = $props();

  // Two notes can share a name in different folders, and a tab has no room for
  // the path — so only the ones that clash carry a folder qualifier (see
  // tabLabels).
  const labels = $derived(tabLabels(tabs));
  const bare = (path: string) => path.replace(/\.md$/, "");
  // A pinned tab's double-click renames the note, so it's worth saying; a
  // preview tab's promotes it, which needs no announcing. A joined tab names
  // both of its notes, and renaming from it would be ambiguous.
  const tip = (tab: Tab) =>
    isJoined(tab)
      ? tab.panes.map((p) => bare(p.path)).join("  ·  ")
      : tab.preview
        ? bare(tab.panes[0].path)
        : `${bare(tab.panes[0].path)}\nDouble-click to rename`;

  let strip = $state<HTMLElement | undefined>(undefined);
  // Whether there are tabs scrolled out of sight on either side. The strip has
  // no scrollbar (one would eat a 26px-tall row), so a fade at that edge is the
  // only thing saying there is more — without it, tabs off-screen read as gone.
  let moreLeft = $state(false);
  let moreRight = $state(false);
  function measure() {
    const el = strip;
    if (!el) return;
    // A pixel of slack: fractional widths never land exactly on the ends.
    moreLeft = el.scrollLeft > 1;
    moreRight = el.scrollLeft + el.clientWidth < el.scrollWidth - 1;
  }

  // Keep the active tab in view when it changes from a hotkey (Cmd+1..9) rather
  // than a click, and re-measure whenever the strip's contents or width change —
  // opening a note, closing one, a rename, or the window resizing.
  $effect(() => {
    const el = strip?.querySelector<HTMLElement>(`[data-tab="${active}"]`);
    el?.scrollIntoView({ block: "nearest", inline: "nearest" });
    labels; // re-run when the tabs themselves change
    measure();
  });
  $effect(() => {
    const el = strip;
    if (!el) return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  });

  function onMouseDown(e: MouseEvent, i: number) {
    // Middle-click closes, as in a browser. Must be mousedown: a middle
    // "click" event isn't dispatched everywhere.
    if (e.button === 1) {
      e.preventDefault();
      onclose(i);
    }
  }

  // ---- Reordering by drag (#266) ----
  // The tab being dragged, and the gap it would drop into — an index *between*
  // tabs (0 … tabs.length), which is what the insertion bar marks. Local state
  // rather than the tree's `$lib/dnd` store: this drag begins and ends inside
  // the strip.
  let dragFrom = $state<number | null>(null);
  let dropGap = $state<number | null>(null);

  function onDragStart(e: DragEvent, i: number) {
    dragFrom = i;
    // WebKit needs setData called or the drag never starts.
    e.dataTransfer?.setData("text/plain", tabs[i].panes[0].path);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
  }
  // The tab a drop would join with, rather than slot beside — set when the
  // pointer is over the middle of another tab (#169).
  let dropOnto = $state<number | null>(null);

  function onDragOverTab(e: DragEvent, i: number) {
    if (dragFrom === null) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
    const box = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const across = (e.clientX - box.left) / box.width;
    // The middle of a tab joins the two notes; the outer thirds are the gaps
    // either side, which reorder. Only where a join is actually possible —
    // two panes is the limit, so over a split (or its own tab) the whole
    // width reorders as before.
    const joinable = i !== dragFrom && !isJoined(tabs[i]) && !isJoined(tabs[dragFrom]);
    if (joinable && across > 0.33 && across < 0.67) {
      dropOnto = i;
      dropGap = null;
      return;
    }
    dropOnto = null;
    dropGap = across > 0.5 ? i + 1 : i;
  }
  function onDrop(e: DragEvent) {
    e.preventDefault();
    const from = dragFrom;
    const gap = dropGap;
    const onto = dropOnto;
    dragFrom = null;
    dropGap = null;
    dropOnto = null;
    if (from === null) return;
    if (onto !== null) {
      onjoin(from, onto);
      return;
    }
    if (gap === null) return;
    // A gap is between tabs; removing the dragged tab first shifts every gap
    // to its right down by one.
    onreorder(from, gap > from ? gap - 1 : gap);
  }
  function endDrag() {
    // Covers a drag dropped outside the strip, which cancels rather than
    // moving anything.
    dragFrom = null;
    dropGap = null;
    dropOnto = null;
  }

  // A trackpad swipes the strip sideways on its own; a mouse wheel only has an
  // axis the strip can't use, so spend it on horizontal scroll (what browsers
  // and editors do over a tab strip).
  function onWheel(e: WheelEvent) {
    const el = strip;
    if (!el || Math.abs(e.deltaX) > Math.abs(e.deltaY)) return;
    if (el.scrollWidth <= el.clientWidth) return;
    e.preventDefault();
    el.scrollLeft += e.deltaY;
  }
</script>

<!-- Tabs live in the header, where the breadcrumb sits on mobile (#169). Desktop
     only: the mobile chrome is built around one note filling the screen.

     The strip is a drag region like the rest of the header: it spans the width
     the header used to, so without this the window loses most of the strip it
     can be dragged by. Tauri hit-tests the element under the pointer, and the
     tabs themselves don't carry the attribute — so they still take their own
     clicks, while the empty space beside them drags the window.

     tabindex="-1" on the strip: it takes drop events (a dragged tab), which
     reads as interactive to the a11y lint, and -1 keeps it out of the tab
     order where the tabs themselves belong. -->
<div
  bind:this={strip}
  data-tauri-drag-region
  class="tab-strip flex min-w-0 flex-1 items-stretch gap-0.5 overflow-x-auto"
  class:fade-l={moreLeft}
  class:fade-r={moreRight}
  role="tablist"
  aria-label="Open notes"
  tabindex={-1}
  onscroll={measure}
  onwheel={onWheel}
  ondragover={(e) => {
    // The empty space past the last tab: dropping there sends it to the end.
    if (dragFrom === null) return;
    e.preventDefault();
    dropOnto = null;
    dropGap = tabs.length;
  }}
  ondrop={onDrop}
>
  {#each tabs as tab, i (tab.panes.map((p) => p.path).join("|"))}
    <!-- The close button is nested inside, so this is a div rather than a
         button (no interactive descendants) with the tab role doing the work. -->
    <!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
    <div
      data-tab={i}
      role="tab"
      tabindex={i === active ? 0 : -1}
      aria-selected={i === active}
      title={tip(tab)}
      draggable="true"
      class="group relative flex max-w-52 shrink-0 cursor-default items-center gap-1 rounded border transition-colors {isMacApp
        ? 'py-0 pl-1.5 pr-0.5'
        : 'py-0.5 pl-2 pr-0.5'} {i === active
        ? 'border-border bg-background text-foreground'
        : 'border-transparent text-muted-foreground hover:bg-muted hover:text-foreground'} {dragFrom ===
      i
        ? 'opacity-40'
        : ''}"
      class:drop-before={dropGap === i && dragFrom !== null}
      class:drop-after={dropGap === i + 1 && dragFrom !== null}
      class:drop-onto={dropOnto === i}
      style="-webkit-user-drag:element;"
      onclick={() => onselect(i)}
      ondblclick={() => ondblclick(i)}
      onmousedown={(e) => onMouseDown(e, i)}
      ondragstart={(e) => onDragStart(e, i)}
      ondragover={(e) => onDragOverTab(e, i)}
      ondrop={onDrop}
      ondragend={endDrag}
      onkeydown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onselect(i);
        }
      }}
    >
      {#if isJoined(tab)}
        <!-- A split reads as one tab naming both notes, with the icon between
             them saying why there are two. -->
        <Columns2 size={12} class="shrink-0 opacity-60" />
      {/if}
      {#each labels[i] as label, p}
        {#if p > 0}
          <span class="shrink-0 text-muted-foreground/40">|</span>
        {/if}
        <!-- Italic marks a preview tab: the one the next sidebar click replaces.
             In a split, the pane with the keyboard is the un-dimmed one. -->
        <span
          class="truncate text-sm {tab.preview ? 'italic' : ''} {isJoined(tab) && p !== tab.focused
            ? 'text-muted-foreground'
            : ''}">{label.name}</span
        >
        {#if label.qualifier}
          <!-- Dimmer and smaller than the name, and the first thing truncated:
               it is here to break a tie, not to be read. -->
          <span class="min-w-0 shrink truncate text-xs text-muted-foreground/70"
            >{label.qualifier}</span
          >
        {/if}
      {/each}
      {#if isJoined(tab)}
        <!-- Put the two notes back in tabs of their own. Hover-revealed like
             the close button, and beside it, since both are "undo this tab". -->
        <button
          type="button"
          class="shrink-0 rounded p-0.5 text-muted-foreground opacity-0 transition-opacity hover:bg-muted hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100"
          aria-label="Split {labels[i].map((l) => l.name).join(' and ')} into separate tabs"
          title="Split into separate tabs"
          onclick={(e) => {
            e.stopPropagation();
            onsplit(i);
          }}
        >
          <Ungroup size={12} />
        </button>
      {/if}
      <!-- Reserved space rather than a mounted-on-hover button, so the tab
           doesn't change width under the pointer as you move along the strip. -->
      <button
        type="button"
        class="shrink-0 rounded p-0.5 text-muted-foreground opacity-0 transition-opacity hover:bg-muted hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100 {i ===
        active
          ? 'opacity-60'
          : ''}"
        aria-label="Close {labels[i].map((l) => l.name).join(' and ')}"
        title={isJoined(tab) ? "Close both" : "Close"}
        onclick={(e) => {
          e.stopPropagation();
          onclose(i);
        }}
      >
        <X size={12} />
      </button>
    </div>
  {/each}
</div>

<style>
  /* No scrollbar: one would eat a 26px-tall row. The strip scrolls by
     trackpad, by the wheel handler above, and by scrollIntoView. */
  .tab-strip {
    scrollbar-width: none;
  }
  .tab-strip::-webkit-scrollbar {
    display: none;
  }
  /* Where a dragged tab would land (#266). Inset rather than sitting in the
     gap, so the bar on the first or last tab isn't clipped by the scroller. */
  .drop-before::before,
  .drop-after::after {
    content: "";
    position: absolute;
    top: 1px;
    bottom: 1px;
    width: 2px;
    border-radius: 1px;
    background: var(--primary);
  }
  .drop-before::before {
    left: 0;
  }
  .drop-after::after {
    right: 0;
  }
  /* The tab a drop would join with: ringed rather than barred, since the two
     notes merge into it instead of slotting beside it. */
  .drop-onto {
    box-shadow: inset 0 0 0 2px var(--primary);
  }
  /* Fade whichever edge has tabs behind it. Masking the strip's own box (not
     its content) means the fade stays at the edge as the content scrolls under
     it. Same idiom as the mobile status-bar scrim. */
  .fade-l {
    -webkit-mask-image: linear-gradient(to right, transparent, #000 2.25rem);
    mask-image: linear-gradient(to right, transparent, #000 2.25rem);
  }
  .fade-r {
    -webkit-mask-image: linear-gradient(to left, transparent, #000 2.25rem);
    mask-image: linear-gradient(to left, transparent, #000 2.25rem);
  }
  .fade-l.fade-r {
    -webkit-mask-image: linear-gradient(
      to right,
      transparent,
      #000 2.25rem,
      #000 calc(100% - 2.25rem),
      transparent
    );
    mask-image: linear-gradient(
      to right,
      transparent,
      #000 2.25rem,
      #000 calc(100% - 2.25rem),
      transparent
    );
  }
</style>
