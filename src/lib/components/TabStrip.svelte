<script lang="ts">
  import { X } from "@lucide/svelte";
  import { isMacApp } from "$lib/platform";
  import { tabLabels, type Tab } from "$lib/tabs";

  let {
    tabs,
    active,
    onselect,
    onclose,
    ondblclick,
  }: {
    tabs: Tab[];
    active: number;
    onselect: (i: number) => void;
    onclose: (i: number) => void;
    // Promotes the tab: pins a preview tab (the same gesture that pins one from
    // the sidebar), renames an already-pinned one.
    ondblclick: (i: number) => void;
  } = $props();

  // Two notes can share a name in different folders, and a tab has no room for
  // the path — so only the ones that clash carry a folder qualifier (see
  // tabLabels).
  const labels = $derived(tabLabels(tabs));
  // A pinned tab's double-click renames the note, so it's worth saying; a
  // preview tab's promotes it, which needs no announcing.
  const tip = (tab: Tab) =>
    tab.preview
      ? tab.path.replace(/\.md$/, "")
      : `${tab.path.replace(/\.md$/, "")}\nDouble-click to rename`;

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
     clicks, while the empty space beside them drags the window. -->
<div
  bind:this={strip}
  data-tauri-drag-region
  class="tab-strip flex min-w-0 flex-1 items-stretch gap-0.5 overflow-x-auto"
  class:fade-l={moreLeft}
  class:fade-r={moreRight}
  role="tablist"
  aria-label="Open notes"
  onscroll={measure}
  onwheel={onWheel}
>
  {#each tabs as tab, i (tab.path)}
    <!-- The close button is nested inside, so this is a div rather than a
         button (no interactive descendants) with the tab role doing the work. -->
    <!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
    <div
      data-tab={i}
      role="tab"
      tabindex={i === active ? 0 : -1}
      aria-selected={i === active}
      title={tip(tab)}
      class="group flex max-w-52 shrink-0 cursor-default items-center gap-1 rounded border transition-colors {isMacApp
        ? 'py-0 pl-1.5 pr-0.5'
        : 'py-0.5 pl-2 pr-0.5'} {i === active
        ? 'border-border bg-background text-foreground'
        : 'border-transparent text-muted-foreground hover:bg-muted hover:text-foreground'}"
      onclick={() => onselect(i)}
      ondblclick={() => ondblclick(i)}
      onmousedown={(e) => onMouseDown(e, i)}
      onkeydown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onselect(i);
        }
      }}
    >
      <!-- Italic marks a preview tab: the one the next sidebar click replaces. -->
      <span class="truncate text-sm {tab.preview ? 'italic' : ''}">{labels[i].name}</span>
      {#if labels[i].qualifier}
        <!-- Dimmer and smaller than the name, and the first thing truncated: it
             is here to break a tie, not to be read. -->
        <span class="min-w-0 shrink truncate text-xs text-muted-foreground/70"
          >{labels[i].qualifier}</span
        >
      {/if}
      <!-- Reserved space rather than a mounted-on-hover button, so the tab
           doesn't change width under the pointer as you move along the strip. -->
      <button
        type="button"
        class="shrink-0 rounded p-0.5 text-muted-foreground opacity-0 transition-opacity hover:bg-muted hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100 {i ===
        active
          ? 'opacity-60'
          : ''}"
        aria-label="Close {labels[i].name}"
        title="Close"
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
