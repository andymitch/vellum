<script lang="ts">
  import { Plus } from "@lucide/svelte";
  // `hidden` slides the button off the bottom edge (auto-hide on scroll, #85);
  // it stays mounted so it animates back in on scroll-up.
  let {
    onclick,
    disabled = false,
    hidden = false,
  }: { onclick: () => void; disabled?: boolean; hidden?: boolean } = $props();
</script>

<button
  type="button"
  {onclick}
  {disabled}
  aria-label="New note"
  title="New note"
  class="fixed bottom-5 right-5 z-30 flex h-14 w-14 items-center justify-center rounded-full bg-primary text-primary-foreground shadow-lg transition-transform duration-200 active:scale-95 disabled:opacity-40 {hidden
    ? 'translate-y-[160%]'
    : ''}"
  style="bottom: calc(1.25rem + env(safe-area-inset-bottom)); right: calc(1.25rem + env(safe-area-inset-right));"
>
  <Plus size={26} />
</button>
