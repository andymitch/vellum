<script lang="ts">
  // Search palette (#15) — Cmd/Ctrl+F. A floating overlay rather than a sidebar
  // filter so it works the same on desktop and mobile, where the sidebar is a
  // drawer. Typing a leading "#" switches to tag mode: pick a tag and it
  // searches for that tag, since a tag filter is just a search for "#tag".
  import { fade, fly } from "svelte/transition";
  import { Search, Hash } from "@lucide/svelte";
  import { searchNotes, listTags, type SearchHit, type TagCount } from "$lib/vault";
  import { portal } from "$lib/portal";

  let {
    open = $bindable(false),
    vault,
    // Initial query — set when opening from a tag chip in the preview.
    initial = "",
    onopen,
  }: {
    open?: boolean;
    vault: string | null;
    initial?: string;
    onopen: (path: string) => void;
  } = $props();

  let query = $state("");
  let hits = $state<SearchHit[]>([]);
  let tags = $state<TagCount[]>([]);
  let selected = $state(0);
  let searching = $state(false);
  // Guards against a slow scan landing after a newer one (results would flicker
  // back to a stale query). Only the latest request may write to `hits`.
  let seq = 0;

  // Tag mode: the query is a bare "#..." and the user is still picking a tag
  // rather than searching for one.
  //
  // Picking a tag used to leave this mode by appending a space to the query,
  // which made the space part of the search needle — and a line ending in a tag
  // never contains "#tag ", so the last tag on every line was unfindable (#202).
  // The mode is tracked separately now so the query stays exactly the tag.
  let mode = $state<"picking" | "searching">("picking");
  const tagMode = $derived(mode === "picking" && /^#\S*$/.test(query));
  const tagFilter = $derived(tagMode ? query.slice(1).toLowerCase() : "");
  const shownTags = $derived(
    tagMode ? tags.filter((t) => t.tag.toLowerCase().includes(tagFilter)) : [],
  );
  const count = $derived(tagMode ? shownTags.length : hits.length);

  // Reset and seed each time the palette opens, so a stale query from last time
  // isn't shown, and load the tag list once for "#" completion.
  $effect(() => {
    if (!open) return;
    query = initial;
    // A seeded query only ever comes from a preview tag chip, which means the
    // tag is already chosen — open straight into results, not the picker.
    mode = initial ? "searching" : "picking";
    selected = 0;
    hits = [];
    if (vault) listTags(vault).then((t) => (tags = t)).catch(() => (tags = []));
  });

  // Debounced: each keystroke would otherwise scan (and CRDT-merge) every note.
  $effect(() => {
    const q = query;
    const v = vault;
    if (!open || tagMode) return;
    if (!v || !q.trim()) {
      hits = [];
      return;
    }
    const mine = ++seq;
    searching = true;
    const t = setTimeout(async () => {
      try {
        const r = await searchNotes(v, q, 50);
        if (mine === seq) {
          hits = r;
          selected = 0;
        }
      } catch {
        if (mine === seq) hits = [];
      } finally {
        if (mine === seq) searching = false;
      }
    }, 150);
    return () => clearTimeout(t);
  });

  function choose(i: number) {
    if (tagMode) {
      const t = shownTags[i];
      if (!t) return;
      // Selecting a tag searches for it. The query stays exactly "#tag" — the
      // backend reads that as a tag query and matches whole tags (#202).
      query = `#${t.tag}`;
      mode = "searching";
      selected = 0;
      return;
    }
    const hit = hits[i];
    if (!hit) return;
    open = false;
    onopen(hit.path);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      open = false;
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      if (count) selected = (selected + 1) % count;
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      if (count) selected = (selected - 1 + count) % count;
    } else if (e.key === "Enter") {
      e.preventDefault();
      choose(selected);
    }
  }

  function focusInput(node: HTMLInputElement) {
    requestAnimationFrame(() => {
      node.focus();
      node.select();
    });
  }

  // Strip the "{line}: " prefix the backend adds, so the number can be shown in
  // its own column rather than inline with the text.
  function splitLine(l: string): { n: string; text: string } {
    const m = /^(\d+): ([\s\S]*)$/.exec(l);
    return m ? { n: m[1], text: m[2] } : { n: "", text: l };
  }

  const displayPath = (p: string) => p.replace(/\.md$/, "");
</script>

{#if open}
  <div
    use:portal
    class="fixed inset-0 z-50 flex items-start justify-center bg-black/50 p-4 pt-[12vh]"
    role="presentation"
    transition:fade={{ duration: 120 }}
    onclick={(e) => {
      if (e.target === e.currentTarget) open = false;
    }}
  >
    <div
      class="flex max-h-[70vh] w-full max-w-xl flex-col overflow-hidden rounded-lg border border-border bg-popover shadow-xl"
      transition:fly={{ y: -8, duration: 140 }}
    >
      <div class="flex items-center gap-2 border-b border-border px-3 py-2.5">
        {#if tagMode}
          <Hash class="h-4 w-4 shrink-0 text-muted-foreground" />
        {:else}
          <Search class="h-4 w-4 shrink-0 text-muted-foreground" />
        {/if}
        <!-- svelte-ignore a11y_autofocus -->
        <input
          use:focusInput
          bind:value={query}
          oninput={() => (mode = "picking")}
          onkeydown={onKeydown}
          placeholder="Search notes, or # for tags"
          autocapitalize="off"
          autocorrect="off"
          autocomplete="off"
          spellcheck="false"
          class="w-full bg-transparent text-sm outline-none placeholder:text-muted-foreground"
        />
      </div>

      <div class="min-h-0 flex-1 overflow-y-auto">
        {#if tagMode}
          {#each shownTags as t, i (t.tag)}
            <button
              class="flex w-full items-center justify-between gap-3 px-3 py-2 text-left hover:bg-muted {i ===
              selected
                ? 'bg-muted'
                : ''}"
              onclick={() => choose(i)}
              onmouseenter={() => (selected = i)}
            >
              <span class="truncate text-sm">#{t.tag}</span>
              <span class="shrink-0 text-xs text-muted-foreground">
                {t.count}
                {t.count === 1 ? "note" : "notes"}
              </span>
            </button>
          {:else}
            <p class="px-3 py-6 text-center text-sm text-muted-foreground">
              {tags.length ? "No matching tags" : "No tags yet — write #atag in a note"}
            </p>
          {/each}
        {:else}
          {#each hits as hit, i (hit.path)}
            <button
              class="block w-full px-3 py-2 text-left hover:bg-muted {i === selected
                ? 'bg-muted'
                : ''}"
              onclick={() => choose(i)}
              onmouseenter={() => (selected = i)}
            >
              <span class="block truncate text-sm">{displayPath(hit.path)}</span>
              {#each hit.lines as line (line)}
                {@const l = splitLine(line)}
                <span class="mt-0.5 flex gap-2 text-xs text-muted-foreground">
                  <span class="shrink-0 tabular-nums opacity-60">{l.n}</span>
                  <span class="truncate">{l.text}</span>
                </span>
              {/each}
            </button>
          {:else}
            <p class="px-3 py-6 text-center text-sm text-muted-foreground">
              {#if !query.trim()}
                Type to search this vault
              {:else if searching}
                Searching…
              {:else}
                No matches
              {/if}
            </p>
          {/each}
        {/if}
      </div>
    </div>
  </div>
{/if}
