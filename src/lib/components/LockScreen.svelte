<script lang="ts">
  // Covers the app while the open vault is private and locked (#274). The app
  // stays mounted (and inert) underneath, so nothing in flight — a save, a
  // scroll position — is lost to the lock.
  import { untrack } from "svelte";
  import { LockKeyhole } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { listVaults } from "$lib/vault";
  import { privacy } from "$lib/private-vaults.svelte";
  import { isMacApp } from "$lib/platform";

  let { vault }: { vault: string } = $props();

  let name = $state("");
  let busy = $state(false);
  let error = $state("");

  $effect(() => {
    const v = vault;
    listVaults()
      .then((all) => (name = all.find((x) => x.id === v)?.name ?? ""))
      .catch(() => {});
  });

  async function unlock() {
    if (busy) return;
    busy = true;
    error = "";
    try {
      if (!(await privacy.unlock(vault))) error = "Not unlocked. Try again.";
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  // Ask straight away when the lock appears in front of someone; a hidden
  // window (a launch to the tray) waits for the button.
  $effect(() => {
    vault;
    if (!document.hidden) untrack(() => void unlock());
  });
</script>

<div
  data-tauri-drag-region
  class="fixed inset-0 z-[100] flex flex-col items-center justify-center gap-4 bg-background px-4 text-foreground"
  style="padding-top:env(safe-area-inset-top);padding-bottom:env(safe-area-inset-bottom);"
>
  <LockKeyhole class="h-10 w-10 text-muted-foreground" />
  <p class="text-center text-sm text-muted-foreground">
    {name ? `“${name}” is private.` : "This vault is private."}
  </p>
  <Button onclick={unlock} disabled={busy}>
    {isMacApp ? "Unlock with Touch ID or password" : "Unlock"}
  </Button>
  {#if error}
    <p class="max-w-sm text-center text-sm text-destructive" role="alert">{error}</p>
  {/if}
</div>
