<script lang="ts">
  // A draggable divider. It reports where the pointer is and nothing else — the
  // host decides what that means, since a split's ratio and a sidebar's width
  // are measured differently (#169, #265).
  //
  // The hit area is wider than the line it draws: a 1px target is a fiddle, and
  // the extra width is transparent, so the seam still reads as a hairline.
  let {
    ondrag,
    onnudge,
    onreset,
    label = "Resize",
    value,
  }: {
    /// The pointer's x, during a drag.
    ondrag: (clientX: number) => void;
    /// Arrow-key resize, in the direction given (-1 left, 1 right).
    onnudge?: (dir: -1 | 1) => void;
    /// Double-click: back to the default position.
    onreset?: () => void;
    label?: string;
    /// Where the divider sits, as a percentage, for assistive tech.
    value?: number;
  } = $props();

  let dragging = $state(false);

  function onPointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    // Stops the drag from selecting text in either pane, and — inside the
    // header's drag region — from moving the window.
    e.preventDefault();
    e.stopPropagation();
    dragging = true;
    // Capture, so the drag keeps working when the pointer outruns the divider.
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function onPointerMove(e: PointerEvent) {
    if (dragging) ondrag(e.clientX);
  }
  function onPointerUp(e: PointerEvent) {
    dragging = false;
    (e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId);
  }
</script>

<!-- A focusable separator is the window-splitter pattern: it is a widget, and
     the arrow keys resize it. The lint rules below only know the static
     separator, which is why they're waived here. -->
<!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  role="separator"
  aria-orientation="vertical"
  aria-label={label}
  aria-valuenow={value === undefined ? undefined : Math.round(value)}
  aria-valuemin={0}
  aria-valuemax={100}
  tabindex="0"
  class="group relative z-10 flex w-1.5 shrink-0 cursor-col-resize touch-none items-stretch justify-center focus-visible:outline-none"
  onpointerdown={onPointerDown}
  onpointermove={onPointerMove}
  onpointerup={onPointerUp}
  onpointercancel={onPointerUp}
  ondblclick={onreset}
  onkeydown={(e) => {
    if (e.key === "ArrowLeft") {
      e.preventDefault();
      onnudge?.(-1);
    } else if (e.key === "ArrowRight") {
      e.preventDefault();
      onnudge?.(1);
    }
  }}
>
  <!-- The line itself: the border colour at rest, the accent while dragged or
       hovered, so it's clear the seam is a control. -->
  <div
    class="w-px transition-colors {dragging
      ? 'bg-primary'
      : 'bg-border group-hover:bg-primary/60 group-focus-visible:bg-primary/60'}"
  ></div>
</div>
