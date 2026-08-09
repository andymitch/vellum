<script lang="ts">
  import { Plus, FileText, ListChecks, NotepadText } from "@lucide/svelte";
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
    scratchpad: NotepadText,
  };

  // Hold-to-pick (#176). Touch only: holding a button isn't a desktop idiom, and
  // a mouse gets the ordinary click.
  const HOLD_MS = 350;
  const RADIUS = 104;
  // Selection is ANGULAR, not distance-to-a-small-circle. The first version
  // required landing within 44px of a 48px target, which is why it barely
  // functioned — a thumb sweeping the arc fell between options and armed
  // nothing. Now anything past a small dead zone picks the nearest option by
  // angle, so the whole quadrant is live and precision stops mattering.
  const DEAD_ZONE = 28;

  let picking = $state(false);
  let pressing = $state(false);
  let active = $state(-1);
  let holdTimer: ReturnType<typeof setTimeout> | undefined;
  // Set when a hold fires, so the click that follows the release is swallowed —
  // otherwise letting go would both pick a type AND create a plain note.
  let suppressClick = false;
  let btn: HTMLButtonElement;

  // Options fan out up and to the left, where a right thumb naturally travels.
  // Index 0 sits directly above the button, the last one directly left.
  function angleFor(i: number): number {
    const step = Math.PI / 2 / Math.max(NOTE_TYPES.length - 1, 1);
    return Math.PI / 2 + i * step; // 90° (up) → 180° (left)
  }
  function offset(i: number): { x: number; y: number } {
    const a = angleFor(i);
    return { x: Math.cos(a) * RADIUS, y: -Math.sin(a) * RADIUS };
  }

  function reset() {
    clearTimeout(holdTimer);
    holdTimer = undefined;
    picking = false;
    pressing = false;
    active = -1;
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
      suppressClick = true;
      // Keep receiving moves once the thumb leaves the button's own box.
      btn?.setPointerCapture?.(e.pointerId);
    }, HOLD_MS);
  }

  function onPointerMove(e: PointerEvent) {
    if (!picking || !btn) return;
    const r = btn.getBoundingClientRect();
    const cx = r.left + r.width / 2;
    const cy = r.top + r.height / 2;
    const dx = e.clientX - cx;
    const dy = cy - e.clientY; // screen y grows downward; flip it for maths
    if (Math.hypot(dx, dy) < DEAD_ZONE) {
      active = -1; // back at the button = nothing armed, so it's an easy cancel
      return;
    }
    const a = Math.atan2(dy, dx);
    let best = 0;
    let bestDiff = Infinity;
    NOTE_TYPES.forEach((_, i) => {
      const d = Math.abs(a - angleFor(i));
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
    {#each NOTE_TYPES as t, i (t.id)}
      {@const o = offset(i)}
      {@const Icon = ICONS[t.id]}
      <div
        class="dial-option pointer-events-none absolute flex h-14 w-14 items-center justify-center rounded-full border shadow-lg transition-colors duration-150 {active ===
        i
          ? 'border-primary bg-primary text-primary-foreground'
          : 'border-border bg-popover text-muted-foreground'}"
        style="left: calc(50% + {o.x}px - 1.75rem); top: calc(50% + {o.y}px - 1.75rem); animation-delay: {i *
          30}ms; {active === i ? 'scale: 1.15;' : ''}"
      >
        <Icon size={22} />
      </div>
    {/each}
    <!-- Name only the armed option: three labels at once is noise, and the icon
         carries the meaning once the set is familiar. -->
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
    class="flex h-14 w-14 items-center justify-center rounded-full bg-primary text-primary-foreground shadow-lg transition-transform duration-200 disabled:opacity-40 {picking
      ? 'scale-90'
      : pressing
        ? 'scale-95'
        : 'active:scale-95'}"
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
  /* Options scale up out of the button rather than snapping into place. */
  .dial-option {
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
