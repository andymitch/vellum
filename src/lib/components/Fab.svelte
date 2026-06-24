<script lang="ts">
  import { Plus } from "@lucide/svelte";
  let { onclick, disabled = false }: { onclick: () => void; disabled?: boolean } =
    $props();
</script>

<button
  type="button"
  {onclick}
  {disabled}
  aria-label="New note"
  title="New note"
  class="fixed bottom-5 right-5 z-30 flex h-14 w-14 items-center justify-center rounded-full bg-primary text-primary-foreground shadow-lg transition-transform active:scale-95 disabled:opacity-40"
  style="bottom: calc(1.25rem + env(safe-area-inset-bottom)); right: calc(1.25rem + env(safe-area-inset-right));"
>
  <Plus size={26} />
</button>
