<script lang="ts">
  import { Plus, FileText, ListChecks, NotebookPen } from "@lucide/svelte";
  import { NOTE_TYPES, type NoteType } from "$lib/note-type";

  // `hidden` slides the button off the bottom edge (auto-hide on scroll, #85);
  // it stays mounted so it animates back in on scroll-up.
  let {
    onclick,
    // Hold the button to pick a note type (#104). Absent = plain button.
    ontype,
    disabled = false,
    hidden = false,
  }: {
    onclick: () => void;
    ontype?: (type: NoteType) => void;
    disabled?: boolean;
    hidden?: boolean;
  } = $props();

  const ICONS: Record<NoteType, typeof FileText> = {
    markdown: FileText,
    todo: ListChecks,
    // Matches the tree (#190) — a bound notebook, not another lined page.
    journal: NotebookPen,
  };

  // Hold-to-pick (#176/#192). Touch only: holding a button isn't a desktop
  // idiom, and a mouse gets the ordinary click.
  const HOLD_MS = 350;
  const RADIUS = 104;
  // Selection is ANGULAR, not distance-to-a-small-circle — a thumb sweeping the
  // arc used to fall between options and arm nothing (#176). Anything past the
  // dead zone picks the nearest option by angle, so the whole quadrant is live.
  const DEAD_ZONE = 28;

  let picking = $state(false);
  let pressing = $state(false);
  let active = $state(-1);
  // Where the thumb is, relative to the button's centre — drives the connector
  // stretched out toward the armed option (#192).
  let reach = $state({ x: 0, y: 0 });
  let holdTimer: ReturnType<typeof setTimeout> | undefined;
  // Set when a hold fires, so the click that follows the release is swallowed —
  // otherwise letting go would both pick a type AND create a plain note.
  let suppressClick = false;
  let btn: HTMLButtonElement;

  // Options fan out up and to the left, where a right thumb naturally travels.
  function angleFor(i: number): number {
    const step = Math.PI / 2 / Math.max(NOTE_TYPES.length - 1, 1);
    return Math.PI / 2 + i * step; // 90° (up) → 180° (left)
  }
  function offset(i: number): { x: number; y: number } {
    const a = angleFor(i);
    return { x: Math.cos(a) * RADIUS, y: -Math.sin(a) * RADIUS };
  }

  // The connector: a rounded bar from the centre toward the thumb, capped at the
  // dial radius so it reaches the target rather than overshooting past it.
  const reachLen = $derived(Math.min(Math.hypot(reach.x, reach.y), RADIUS));
  const reachDeg = $derived((Math.atan2(reach.y, reach.x) * 180) / Math.PI);

  function reset() {
    clearTimeout(holdTimer);
    holdTimer = undefined;
    picking = false;
    pressing = false;
    active = -1;
    reach = { x: 0, y: 0 };
  }

  function onPointerDown(e: PointerEvent) {
    if (disabled || !ontype || e.pointerType !== "touch") return;
    clearTimeout(holdTimer);
    pressing = true; // immediate feedback that a hold is being registered
    holdTimer = setTimeout(() => {
      picking = true;
      // Nothing armed until the thumb moves, so releasing straight away cancels
      // rather than picking whichever option happened to be first.
      active = -1;
      reach = { x: 0, y: 0 };
      suppressClick = true;
      btn?.setPointerCapture?.(e.pointerId);
    }, HOLD_MS);
  }

  function onPointerMove(e: PointerEvent) {
    if (!picking || !btn) return;
    const r = btn.getBoundingClientRect();
    const cx = r.left + r.width / 2;
    const cy = r.top + r.height / 2;
    const dx = e.clientX - cx;
    const dy = e.clientY - cy;
    reach = { x: dx, y: dy };
    if (Math.hypot(dx, dy) < DEAD_ZONE) {
      active = -1; // back at the button = nothing armed, an easy cancel
      return;
    }
    // Maths convention (y up). Flip at the source rather than negating dy: with
    // the thumb exactly level, `-dy` is NEGATIVE zero, and atan2(-0, negative)
    // returns -π instead of +π — which armed the first option when sweeping
    // straight left, the most natural path to the last one.
    const a = Math.atan2(cy - e.clientY, dx);
    let best = 0;
    let bestDiff = Infinity;
    NOTE_TYPES.forEach((_, i) => {
      // Shortest angular distance, wrapped into [-π, π]. Without the wrap, a
      // thumb just below horizontal (a ≈ -π) reads as maximally far from the
      // last option (≈ +π) when it is in fact adjacent to it.
      const raw = a - angleFor(i);
      const d = Math.abs(Math.atan2(Math.sin(raw), Math.cos(raw)));
      if (d < bestDiff) {
        bestDiff = d;
        best = i;
      }
    });
    active = best;
  }

  function onPointerUp() {
    if (!picking) {
      reset();
      return;
    }
    const chosen = active >= 0 ? NOTE_TYPES[active] : undefined;
    reset();
    // Releasing in the dead zone cancels — a hold you thought better of
    // shouldn't create anything.
    if (chosen) ontype?.(chosen.id);
  }
</script>

<!-- The dial is anchored to the button, so both live in one fixed container. -->
<div
  class="fixed bottom-5 right-5 z-30"
  style="bottom: calc(1.25rem + env(safe-area-inset-bottom)); right: calc(1.25rem + env(safe-area-inset-right));"
>
  {#if picking}
    <!-- Dim the page so the dial reads as a mode, not decoration. -->
    <div class="fixed inset-0 -z-10 bg-black/30"></div>

    <!-- The gooey filter, applied ONLY to the shapes layer below. Blurring and
         re-thresholding an icon would destroy it, so the icons sit in a separate
         unfiltered layer on top. -->
    <svg width="0" height="0" class="absolute" aria-hidden="true">
      <defs>
        <filter id="vellum-dial-goo">
          <feGaussianBlur in="SourceGraphic" stdDeviation="8" result="blur" />
          <!-- Re-sharpen the alpha so blurred edges snap back to solid shapes —
               that ramp is what makes touching blobs merge into one. -->
          <feColorMatrix
            in="blur"
            mode="matrix"
            values="1 0 0 0 0  0 1 0 0 0  0 0 1 0 0  0 0 0 20 -9"
          />
        </filter>
      </defs>
    </svg>

    <!-- Shapes: option blobs plus the connector being dragged out. -->
    <div class="pointer-events-none absolute inset-0" style="filter: url(#vellum-dial-goo);">
      <!-- Anchor blob under the button, so the connector always has something to
           merge with at its origin. -->
      <div
        class="absolute h-14 w-14 rounded-full bg-primary"
        style="left: calc(50% - 1.75rem); top: calc(50% - 1.75rem);"
      ></div>

      {#if reachLen > DEAD_ZONE / 2}
        <div
          class="absolute h-10 rounded-full bg-primary"
          style="left: 50%; top: calc(50% - 1.25rem); width: {reachLen}px; transform-origin: 0 50%; transform: rotate({reachDeg}deg);"
        ></div>
      {/if}

      {#each NOTE_TYPES as t, i (t.id)}
        {@const o = offset(i)}
        <div
          class="dial-blob absolute h-14 w-14 rounded-full transition-colors duration-150 {active ===
          i
            ? 'bg-primary'
            : 'bg-popover'}"
          style="left: calc(50% + {o.x}px - 1.75rem); top: calc(50% + {o.y}px - 1.75rem); animation-delay: {i *
            30}ms;"
        ></div>
      {/each}
    </div>

    <!-- Icons: identical positions, no filter. -->
    <div class="pointer-events-none absolute inset-0">
      {#each NOTE_TYPES as t, i (t.id)}
        {@const o = offset(i)}
        {@const Icon = ICONS[t.id]}
        <div
          class="absolute flex h-14 w-14 items-center justify-center transition-colors duration-150 {active ===
          i
            ? 'text-primary-foreground'
            : 'text-muted-foreground'}"
          style="left: calc(50% + {o.x}px - 1.75rem); top: calc(50% + {o.y}px - 1.75rem);"
        >
          <Icon size={22} />
        </div>
      {/each}
    </div>

    <!-- Name only the armed option: three labels at once is noise. -->
    {#if active >= 0}
      <div
        class="pointer-events-none absolute whitespace-nowrap rounded-full bg-popover px-2.5 py-1 text-xs font-medium shadow-lg"
        style="left: 50%; top: calc(50% - {RADIUS + 56}px); transform: translateX(-50%);"
      >
        {NOTE_TYPES[active].label}
      </div>
    {/if}
  {/if}

  <button
    bind:this={btn}
    type="button"
    onclick={() => {
      // Swallow the click that follows a hold — the type was already chosen.
      if (suppressClick) {
        suppressClick = false;
        return;
      }
      onclick();
    }}
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
    onpointercancel={reset}
    oncontextmenu={(e) => picking && e.preventDefault()}
    {disabled}
    aria-label="New note"
    title="New note"
    class="relative flex h-14 w-14 items-center justify-center rounded-full text-primary-foreground shadow-lg transition-transform duration-200 disabled:opacity-40 {picking
      ? 'scale-90 bg-transparent shadow-none'
      : pressing
        ? 'scale-95 bg-primary'
        : 'bg-primary active:scale-95'}"
    style="transform: {hidden
      ? 'translateY(calc(100% + 1.5rem + env(safe-area-inset-bottom)))'
      : 'none'};"
  >
    <Plus
      size={26}
      class="transition-transform duration-150 {picking ? 'rotate-45' : ''}"
    />
  </button>
</div>

<style>
  /* Options grow out of the button rather than snapping into place. */
  .dial-blob {
    animation: dial-in 150ms ease-out backwards;
  }
  @keyframes dial-in {
    from {
      opacity: 0;
      scale: 0.4;
    }
    to {
      opacity: 1;
    }
  }
</style>
