<script lang="ts">
  // The menu-bar popup (#288): one vault's scratch pad and a switcher between
  // vaults, nothing else. It is a second window over the same backend, so it
  // shares the vaults, sync and private-vault locks with the main window.
  import { onMount } from "svelte";
  import NotePane from "$lib/components/editor/NotePane.svelte";
  import LockScreen from "$lib/components/LockScreen.svelte";
  import { listVaults, watchVault, type VaultInfo } from "$lib/vault";
  import { initPrivateVaults, privacy } from "$lib/private-vaults.svelte";
  import { session } from "$lib/session.svelte";
  import { SCRATCH_PAD } from "$lib/note-type";

  const KEY = "vellum-popup-vault";
  let vaults = $state<VaultInfo[]>([]);
  let vault = $state<string | null>(localStorage.getItem(KEY) ?? session.vault);

  // Re-listed each time the popup is shown (it takes focus then), since it
  // stays alive, hidden, between uses while vaults come and go in the main window.
  async function refresh() {
    vaults = await listVaults().catch(() => []);
    if (!vaults.some((v) => v.id === vault)) vault = vaults[0]?.id ?? null;
  }

  onMount(() => {
    void initPrivateVaults();
    void refresh();
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  });

  $effect(() => {
    if (!vault) return;
    localStorage.setItem(KEY, vault);
    watchVault(vault).catch((e) => console.error("could not watch vault", e));
  });
</script>

<div class="flex h-screen w-screen flex-col overflow-hidden bg-background text-foreground">
  <header class="flex shrink-0 items-center gap-2 border-b border-border px-3 py-2">
    <span class="text-sm font-medium">Scratch Pad</span>
    {#if vaults.length > 1}
      <select
        bind:value={vault}
        aria-label="Vault"
        class="ml-auto min-w-0 rounded border border-border bg-background px-1.5 py-0.5 text-xs text-muted-foreground"
      >
        {#each vaults as v (v.id)}
          <option value={v.id}>{v.name}</option>
        {/each}
      </select>
    {/if}
  </header>
  <!-- contain:paint keeps LockScreen's fixed overlay inside this box, so the
       switcher above stays usable on a locked vault. -->
  <div class="relative flex min-h-0 flex-1 flex-col [contain:paint]">
    {#if vault && privacy.ready && !privacy.locked(vault)}
      <NotePane {vault} path={SCRATCH_PAD} mode="source" userScrolling={() => false} />
    {:else if vault && privacy.ready}
      <LockScreen {vault} />
    {:else if !vaults.length}
      <p class="p-4 text-sm text-muted-foreground">Open Vellum to create a vault.</p>
    {/if}
  </div>
</div>
