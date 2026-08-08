<script lang="ts">
  import { Plus } from "@lucide/svelte";
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

  // Hold-to-pick (#104). Touch only: a mouse gets the ordinary click, since
  // holding a button is not a desktop idiom and would only surprise people.
  const HOLD_MS = 400;
  // How far from an option's centre still counts as choosing it. Generous,
  // because a thumb is imprecise and the options are small.
  const HIT_RADIUS = 44;

  let picking = $state(false);
  let active = $state(-1);
  let holdTimer: ReturnType<typeof setTimeout> | undefined;
  // Set when a hold fires, so the click that follows the release is swallowed —
  // otherwise letting go would both pick a type AND create a plain note.
  let suppressClick = false;
  let btn: HTMLButtonElement;

  // Options fan out up and to the left of the button, which is where a right
  // thumb naturally travels. Index 0 sits directly above.
  const RADIUS = 96;
  function offset(i: number): { x: number; y: number } {
    const step = Math.PI / 2 / Math.max(NOTE_TYPES.length - 1, 1);
    const angle = Math.PI / 2 + i * step; // 90° (up) → 180° (left)
    return { x: Math.cos(angle) * RADIUS, y: -Math.sin(angle) * RADIUS };
  }

  function cancelHold() {
    clearTimeout(holdTimer);
    holdTimer = undefined;
    picking = false;
    active = -1;
  }

  function onPointerDown(e: PointerEvent) {
    if (disabled || !ontype || e.pointerType !== "touch") return;
    clearTimeout(holdTimer);
    holdTimer = setTimeout(() => {
      picking = true;
      active = -1;
      suppressClick = true;
      // Keep receiving moves even if the thumb leaves the button's own box.
      btn?.setPointerCapture?.(e.pointerId);
    }, HOLD_MS);
  }

  function onPointerMove(e: PointerEvent) {
    if (!picking || !btn) return;
    const r = btn.getBoundingClientRect();
    const cx = r.left + r.width / 2;
    const cy = r.top + r.height / 2;
    let best = -1;
    let bestDist = HIT_RADIUS;
    NOTE_TYPES.forEach((_, i) => {
      const o = offset(i);
      const d = Math.hypot(e.clientX - (cx + o.x), e.clientY - (cy + o.y));
      if (d < bestDist) {
        bestDist = d;
        best = i;
      }
    });
    active = best;
  }

  function onPointerUp() {
    if (!picking) {
      clearTimeout(holdTimer);
      holdTimer = undefined;
      return;
    }
    const chosen = active >= 0 ? NOTE_TYPES[active] : undefined;
    cancelHold();
    // Releasing away from every option cancels — a hold you thought better of
    // shouldn't create anything.
    if (chosen) ontype?.(chosen.id);
  }
</script>

<!-- The picker is anchored to the button, so both live in one fixed container. -->
<div
  class="fixed bottom-5 right-5 z-30"
  style="bottom: calc(1.25rem + env(safe-area-inset-bottom)); right: calc(1.25rem + env(safe-area-inset-right));"
>
  {#if picking}
    <!-- Dim the page so the fanned-out options read as a mode, not decoration. -->
    <div class="fixed inset-0 -z-10 bg-black/20"></div>
    {#each NOTE_TYPES as t, i (t.id)}
      {@const o = offset(i)}
      <div
        class="pointer-events-none absolute flex h-12 w-12 items-center justify-center rounded-full border text-center text-[10px] leading-tight shadow-lg transition-transform {active ===
        i
          ? 'border-primary bg-primary text-primary-foreground scale-110'
          : 'border-border bg-popover text-muted-foreground'}"
        style="left: calc(50% + {o.x}px - 1.5rem); top: calc(50% + {o.y}px - 1.5rem);"
      >
        {t.label}
      </div>
    {/each}
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
    onpointercancel={cancelHold}
    oncontextmenu={(e) => picking && e.preventDefault()}
    {disabled}
    aria-label="New note"
    title="New note"
    class="flex h-14 w-14 items-center justify-center rounded-full bg-primary text-primary-foreground shadow-lg transition-transform duration-200 active:scale-95 disabled:opacity-40"
    style="transform: {hidden
      ? 'translateY(calc(100% + 1.5rem + env(safe-area-inset-bottom)))'
      : 'none'};"
  >
    <Plus size={26} />
  </button>
</div>
