<script lang="ts">
  import { X } from "@lucide/svelte";
  import { isMacApp } from "$lib/platform";
  import type { Tab } from "$lib/tabs";

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

  const label = (path: string) => path.split("/").pop()!.replace(/\.md$/, "");
  // A pinned tab's double-click renames the note, so it's worth saying; a
  // preview tab's promotes it, which needs no announcing.
  const tip = (tab: Tab) =>
    tab.preview
      ? tab.path.replace(/\.md$/, "")
      : `${tab.path.replace(/\.md$/, "")}\nDouble-click to rename`;

  // Keep the active tab in view when it changes from a hotkey (Cmd+1..9) rather
  // than a click, and when a rename pushes the strip around.
  let strip = $state<HTMLElement | undefined>(undefined);
  $effect(() => {
    const el = strip?.querySelector<HTMLElement>(`[data-tab="${active}"]`);
    el?.scrollIntoView({ block: "nearest", inline: "nearest" });
  });

  function onMouseDown(e: MouseEvent, i: number) {
    // Middle-click closes, as in a browser. Must be mousedown: a middle
    // "click" event isn't dispatched everywhere.
    if (e.button === 1) {
      e.preventDefault();
      onclose(i);
    }
  }
</script>

<!-- Tabs live in the header, where the breadcrumb sits on mobile (#169). Desktop
     only: the mobile chrome is built around one note filling the screen. -->
<div
  bind:this={strip}
  class="tab-strip flex min-w-0 flex-1 items-stretch gap-0.5 overflow-x-auto"
  role="tablist"
  aria-label="Open notes"
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
      <span class="truncate text-sm {tab.preview ? 'italic' : ''}">{label(tab.path)}</span>
      <!-- Reserved space rather than a mounted-on-hover button, so the tab
           doesn't change width under the pointer as you move along the strip. -->
      <button
        type="button"
        class="shrink-0 rounded p-0.5 text-muted-foreground opacity-0 transition-opacity hover:bg-muted hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100 {i ===
        active
          ? 'opacity-60'
          : ''}"
        aria-label="Close {label(tab.path)}"
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
  /* A scrollbar in a 28px-tall strip eats the labels; the strip scrolls by
     wheel/trackpad and by the scrollIntoView above. */
  .tab-strip {
    scrollbar-width: none;
  }
  .tab-strip::-webkit-scrollbar {
    display: none;
  }
</style>
