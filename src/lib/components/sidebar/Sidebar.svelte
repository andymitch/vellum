<script lang="ts">
    import { onMount } from "svelte";
    import {
        listVaults,
        createVault,
        joinVault,
        shareVault,
        forgetVault,
        listTree,
        createNote,
        writeNote,
        createFolder,
        renamePath,
        deletePath,
        watchVault,
        onVaultChanged,
        type VaultInfo,
        type TreeNode,
    } from "$lib/vault";
    import Tree from "./Tree.svelte";
    import { fly, fade } from "svelte/transition";
    import {
        FilePlus,
        FolderPlus,
        Share2,
        Download,
        Plus,
        Trash2,
        ChevronsUpDown,
        Check,
        X,
        ScanLine,
        Settings,
    } from "@lucide/svelte";
    import QRCode from "qrcode";
    import {
        scan,
        checkPermissions,
        requestPermissions,
    } from "@tauri-apps/plugin-barcode-scanner";
    import { theme, PALETTES, FONTS, type Mode } from "$lib/theme.svelte";

    const MODES: { id: Mode; label: string }[] = [
        { id: "system", label: "System" },
        { id: "light", label: "Light" },
        { id: "dark", label: "Dark" },
    ];
    let settingsSheet = $state(false);

    // The native camera scanner only exists on mobile; gate the Scan button on it.
    const canScan = /Android|iPhone|iPad|iPod/i.test(navigator.userAgent);

    async function scanQr(): Promise<string | null> {
        try {
            let perm = await checkPermissions();
            if (perm !== "granted") perm = await requestPermissions();
            if (perm !== "granted") return null;
            const res = await scan({ windowed: false });
            return res?.content?.trim() || null;
        } catch {
            return null; // cancelled or unsupported — fall back to paste
        }
    }

    let {
        activePath = null,
        onopen,
        onvaultchange,
    }: {
        activePath?: string | null;
        onopen: (vault: string, path: string, selectTitle?: boolean) => void;
        onvaultchange: (vault: string | null) => void;
    } = $props();

    // Focus (and select) an input on mount — webview autofocus is unreliable.
    function focusSelect(node: HTMLInputElement) {
        requestAnimationFrame(() => {
            node.focus();
            node.select();
        });
    }

    let vaults = $state<VaultInfo[]>([]);
    let activeVault = $state<string | null>(null);
    let tree = $state<TreeNode[]>([]);
    let expanded = $state<Record<string, boolean>>({});
    let menu = $state<{ x: number; y: number; node: TreeNode } | null>(null);
    let vaultSheet = $state(false);

    const activeVaultName = $derived(
        vaults.find((v) => v.id === activeVault)?.name ??
            (vaults.length ? "Select vault" : "No vaults"),
    );

    // In-app dialogs. wry only implements window.prompt/confirm/alert on Android
    // (no macOS WKWebView impl), so those silently no-op on desktop. Use our own.
    type Dialog =
        | {
              kind: "text";
              title: string;
              value: string;
              resolve: (v: string | null) => void;
          }
        | { kind: "confirm"; title: string; resolve: (v: boolean) => void }
        | { kind: "join"; value: string; resolve: (v: string | null) => void }
        | {
              kind: "share";
              title: string;
              value: string;
              qr: string;
              resolve: () => void;
          };
    let dialog = $state<Dialog | null>(null);

    const askText = (title: string, value = ""): Promise<string | null> =>
        new Promise(
            (resolve) => (dialog = { kind: "text", title, value, resolve }),
        );
    const askJoin = (): Promise<string | null> =>
        new Promise(
            (resolve) => (dialog = { kind: "join", value: "", resolve }),
        );
    const askConfirm = (title: string): Promise<boolean> =>
        new Promise(
            (resolve) => (dialog = { kind: "confirm", title, resolve }),
        );
    const showShare = async (value: string): Promise<void> => {
        // EC level "L" — the ticket is the only payload and is shown alongside the
        // QR, so we trade redundancy for a less-dense, easier-to-scan code.
        const qr = await QRCode.toDataURL(value, {
            margin: 1,
            width: 220,
            errorCorrectionLevel: "L",
        }).catch(() => "");
        return new Promise(
            (resolve) =>
                (dialog = {
                    kind: "share",
                    title: "Vault ticket",
                    value,
                    qr,
                    resolve,
                }),
        );
    };

    // Reparent overlays to <body>. The drawer's translate-x transform makes the
    // aside a containing block for `position: fixed`, which would otherwise trap
    // these overlays inside the 16rem drawer instead of filling the viewport.
    function portal(node: HTMLElement) {
        document.body.appendChild(node);
        return { destroy: () => node.remove() };
    }

    function resolveDialog(result: string | boolean | null) {
        const d = dialog;
        dialog = null;
        if (!d) return;
        if (d.kind === "text" || d.kind === "join")
            d.resolve(result as string | null);
        else if (d.kind === "confirm") d.resolve(result as boolean);
        else d.resolve();
    }

    const dirOf = (path: string) => path.split("/").slice(0, -1).join("/");
    const join = (dir: string, name: string) => (dir ? `${dir}/${name}` : name);

    async function refreshVaults() {
        vaults = await listVaults();
    }
    async function refreshTree() {
        tree = activeVault ? await listTree(activeVault) : [];
    }

    async function setActive(id: string | null) {
        activeVault = id;
        expanded = {};
        onvaultchange(id);
        if (id) {
            await watchVault(id);
            await refreshTree();
        } else {
            tree = [];
        }
    }

    async function newVault() {
        const name = (await askText("New vault name"))?.trim();
        if (!name) return;
        const v = await createVault(name);
        await refreshVaults();
        await setActive(v.id);
        vaultSheet = false;
    }

    async function joinPrompt() {
        const ticket = (await askJoin())?.trim();
        if (!ticket) return;
        const v = await joinVault(ticket);
        await refreshVaults();
        await setActive(v.id);
        vaultSheet = false;
    }

    async function selectVault(id: string) {
        vaultSheet = false;
        if (id !== activeVault) await setActive(id);
    }

    async function share(id: string) {
        const ticket = await shareVault(id);
        try {
            await navigator.clipboard.writeText(ticket);
        } catch {
            /* clipboard may be unavailable; ticket is shown + selectable */
        }
        await showShare(ticket);
    }

    async function forget(id: string) {
        const v = vaults.find((x) => x.id === id);
        if (
            !(await askConfirm(
                `Forget "${v?.name ?? "vault"}"? It stays on other devices and can be rejoined from a ticket.`,
            ))
        )
            return;
        try {
            await forgetVault(id);
        } catch (e) {
            console.error("forget vault failed", e);
            return;
        }
        await refreshVaults();
        if (id === activeVault) await setActive(vaults[0]?.id ?? null);
    }

    // Create a note (named from its first H1, Obsidian-style). Prepopulate the H1
    // with the unique "Untitled" name and open it with that title preselected so
    // the user can immediately type a real name.
    async function newNoteIn(vault: string, dir: string) {
        const path = await createNote(vault, join(dir, "Untitled.md"));
        const title = path.split("/").pop()!.replace(/\.md$/, "");
        const finalPath = await writeNote(vault, path, `# ${title}\n`);
        await refreshTree();
        onopen(vault, finalPath, true);
    }

    async function newRootNote() {
        if (!activeVault) return;
        await newNoteIn(activeVault, "");
    }

    async function newRootFolder() {
        if (!activeVault) return;
        const name = (await askText("New folder name"))?.trim();
        if (!name) return;
        await createFolder(activeVault, name);
        await refreshTree();
    }

    function openMenu(e: MouseEvent, node: TreeNode) {
        e.preventDefault();
        menu = { x: e.clientX, y: e.clientY, node };
    }
    const closeMenu = () => (menu = null);

    async function act(kind: string, node: TreeNode) {
        closeMenu();
        if (!activeVault) return;
        if (kind === "new-note") {
            expanded[node.path] = true;
            await newNoteIn(activeVault, node.path);
            return;
        } else if (kind === "new-folder") {
            const name = (await askText("New folder name"))?.trim();
            if (!name) return;
            await createFolder(activeVault, join(node.path, name));
            expanded[node.path] = true;
        } else if (kind === "rename") {
            const name = (await askText("Rename to", node.name))?.trim();
            if (!name || name === node.name) return;
            const to = join(dirOf(node.path), name);
            await renamePath(activeVault, node.path, to, node.is_dir);
            if (activePath === node.path) onopen(activeVault, to);
        } else if (kind === "delete") {
            if (!(await askConfirm(`Delete "${node.name}"?`))) return;
            await deletePath(activeVault, node.path, node.is_dir);
            if (
                activePath &&
                (activePath === node.path ||
                    activePath.startsWith(node.path + "/"))
            )
                onvaultchange(activeVault);
        }
        await refreshTree();
    }

    const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

    onMount(() => {
        let unlisten: (() => void) | undefined;
        (async () => {
            for (let i = 0; i < 40; i++) {
                try {
                    await refreshVaults();
                    break;
                } catch {
                    await sleep(250);
                }
            }
            if (vaults.length) await setActive(vaults[0].id);
            unlisten = await onVaultChanged((id) => {
                if (id !== activeVault) return;
                refreshTree();
                // The vault name lives in the doc (\x00meta/name) and arrives via
                // sync after a join — re-read names so the fallback "vault-xxxx"
                // updates to the real name once it lands.
                refreshVaults();
            });
        })();
        return () => unlisten?.();
    });
</script>

<svelte:window onclick={closeMenu} />

<div class="flex h-full flex-col">
    <!-- Root file/folder actions -->
    <div class="flex items-center justify-between px-2 py-2 md:py-1.5">
        <div class="flex items-center pl-1">
            <svg
                viewBox="0 0 1000 1000"
                class="h-6 w-6 text-foreground md:h-5.5 md:w-5.5"
                fill="currentColor"
                aria-hidden="true"
            >
                <path
                    d="M376.75,213.17l165,94.58a21.77,21.77,0,0,1,11,18.86V639.16a2.68,2.68,0,0,1-1.39,2.35L372.59,742.86a2.71,2.71,0,0,1-3.7-1h0a2.68,2.68,0,0,1-.36-1.35l.18-522.66a5.44,5.44,0,0,1,5.35-5.29A5.23,5.23,0,0,1,376.75,213.17Zm-8-20.45a8.16,8.16,0,0,0-12.21,7.07l-.79,547.63a10.87,10.87,0,0,0,5.46,9.43L552.59,866.64a8.15,8.15,0,0,0,12.2-7.08V320.93a27.18,27.18,0,0,0-13.63-23.56Zm300.64,60.17V831.16a27.15,27.15,0,0,1-13.68,23.58l-76.38,43.67a27.23,27.23,0,0,1-27.06,0l-208.2-120a27.23,27.23,0,0,1-13.59-23.53V162.19a27.16,27.16,0,0,1,14.27-23.91l68.7-37a27.19,27.19,0,0,1,26.72.52L656.12,229.56A27.18,27.18,0,0,1,669.42,252.89Z"
                />
            </svg>
            <span
                class="font-vellum text-xl font-bold tracking-tight text-foreground md:text-base"
                >Vellum</span
            >
        </div>
        <div class="flex gap-0.5">
            <button
                type="button"
                class="rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground disabled:opacity-40 md:p-1"
                title="New note"
                disabled={!activeVault}
                onclick={newRootNote}
            >
                <FilePlus class="h-4.5 w-4.5 md:h-3.75 md:w-3.75" />
            </button>
            <button
                type="button"
                class="rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground disabled:opacity-40 md:p-1"
                title="New folder"
                disabled={!activeVault}
                onclick={newRootFolder}
            >
                <FolderPlus class="h-4.5 w-4.5 md:h-3.75 md:w-3.75" />
            </button>
        </div>
    </div>

    <!-- Tree -->
    <div class="min-h-0 flex-1 overflow-auto px-1 pb-2">
        {#if activeVault && tree.length}
            <Tree
                nodes={tree}
                {activePath}
                {expanded}
                onselect={(node) => onopen(activeVault!, node.path)}
                onmenu={openMenu}
            />
        {:else if activeVault}
            <p class="px-2 py-4 text-sm text-muted-foreground">Empty vault.</p>
        {:else}
            <p class="px-2 py-4 text-sm text-muted-foreground">
                Create or join a vault to begin.
            </p>
        {/if}
    </div>

    <!-- Vault selector + settings (bottom) -->
    <div class="flex items-center border-t border-border">
        <button
            type="button"
            class="flex min-w-0 flex-1 items-center gap-2 px-3 py-3 text-left text-sm font-medium hover:bg-muted md:py-2.5"
            onclick={() => (vaultSheet = true)}
        >
            <ChevronsUpDown class="h-4 w-4 shrink-0 text-muted-foreground" />
            <span class="truncate">{activeVaultName}</span>
        </button>
        <button
            type="button"
            class="shrink-0 px-3 py-3 text-muted-foreground hover:bg-muted hover:text-foreground md:py-2.5"
            aria-label="Settings"
            title="Settings"
            onclick={() => (settingsSheet = true)}
        >
            <Settings class="h-4.5 w-4.5 md:h-4 md:w-4" />
        </button>
    </div>
</div>

<!-- Manage-vaults: bottom sheet on mobile, centered modal on desktop -->
{#if vaultSheet}
    <div
        use:portal
        class="fixed inset-0 z-50 flex items-end justify-center bg-black/50 md:items-center"
        role="presentation"
        transition:fade={{ duration: 150 }}
        onclick={(e) => {
            if (e.target === e.currentTarget) vaultSheet = false;
        }}
    >
        <div
            class="flex max-h-[80vh] w-full flex-col rounded-t-2xl border border-border bg-popover md:max-w-md md:rounded-2xl"
            style="padding-bottom:env(safe-area-inset-bottom);"
            transition:fly={{ y: 320, duration: 220, opacity: 1 }}
        >
            <!-- Header -->
            <div
                class="flex items-center justify-between border-b border-border px-4 py-3"
            >
                <h2 class="text-base font-semibold">Vaults</h2>
                <button
                    type="button"
                    class="rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground"
                    aria-label="Close"
                    onclick={() => (vaultSheet = false)}
                >
                    <X class="h-5 w-5" />
                </button>
            </div>

            <!-- Vault list -->
            <div class="min-h-0 flex-1 overflow-auto py-1">
                {#if !vaults.length}
                    <p
                        class="px-4 py-6 text-center text-sm text-muted-foreground"
                    >
                        No vaults yet. Create or join one below.
                    </p>
                {/if}
                {#each vaults as v (v.id)}
                    <div class="group flex items-center gap-1 px-2">
                        <button
                            type="button"
                            class="flex min-w-0 flex-1 items-center gap-2 rounded-md px-2 py-2.5 text-left text-sm hover:bg-muted {v.id ===
                            activeVault
                                ? 'font-medium text-foreground'
                                : 'text-muted-foreground'}"
                            onclick={() => selectVault(v.id)}
                        >
                            <Check
                                class="h-4 w-4 shrink-0 {v.id === activeVault
                                    ? 'opacity-100'
                                    : 'opacity-0'}"
                            />
                            <span class="truncate">{v.name}</span>
                        </button>
                        <button
                            type="button"
                            class="rounded p-2 text-muted-foreground hover:bg-muted hover:text-foreground"
                            title="Share vault"
                            aria-label="Share {v.name}"
                            onclick={() => share(v.id)}
                        >
                            <Share2 class="h-4.5 w-4.5" />
                        </button>
                        <button
                            type="button"
                            class="rounded p-2 text-muted-foreground hover:bg-muted hover:text-destructive"
                            title="Forget vault"
                            aria-label="Forget {v.name}"
                            onclick={() => forget(v.id)}
                        >
                            <Trash2 class="h-4.5 w-4.5" />
                        </button>
                    </div>
                {/each}
            </div>

            <!-- Actions -->
            <div class="flex gap-2 border-t border-border p-3">
                <button
                    type="button"
                    class="flex flex-1 items-center justify-center gap-2 rounded-md bg-primary px-3 py-2.5 text-sm font-medium text-primary-foreground hover:bg-primary/90"
                    onclick={newVault}
                >
                    <Plus class="h-4 w-4" /> New vault
                </button>
                <button
                    type="button"
                    class="flex flex-1 items-center justify-center gap-2 rounded-md border border-border px-3 py-2.5 text-sm font-medium hover:bg-muted"
                    onclick={joinPrompt}
                >
                    <Download class="h-4 w-4" /> Join vault
                </button>
            </div>
        </div>
    </div>
{/if}

<!-- Settings: bottom sheet on mobile, centered modal on desktop -->
{#if settingsSheet}
    <div
        use:portal
        class="fixed inset-0 z-50 flex items-end justify-center bg-black/50 md:items-center"
        role="presentation"
        transition:fade={{ duration: 150 }}
        onclick={(e) => {
            if (e.target === e.currentTarget) settingsSheet = false;
        }}
    >
        <div
            class="flex max-h-[80vh] w-full flex-col rounded-t-2xl border border-border bg-popover md:max-w-md md:rounded-2xl"
            style="padding-bottom:env(safe-area-inset-bottom);"
            transition:fly={{ y: 320, duration: 220, opacity: 1 }}
        >
            <div
                class="flex items-center justify-between border-b border-border px-4 py-3"
            >
                <h2 class="text-base font-semibold">Settings</h2>
                <button
                    type="button"
                    class="rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground"
                    aria-label="Close"
                    onclick={() => (settingsSheet = false)}
                >
                    <X class="h-5 w-5" />
                </button>
            </div>

            <div class="min-h-0 flex-1 overflow-auto p-4">
                <!-- Appearance: mode -->
                <p
                    class="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground"
                >
                    Appearance
                </p>
                <div
                    class="mb-4 inline-flex rounded-lg border border-border bg-background p-0.5"
                >
                    {#each MODES as m (m.id)}
                        <button
                            type="button"
                            class="rounded-md px-3 py-1.5 text-sm transition-colors {theme.mode ===
                            m.id
                                ? 'bg-primary text-primary-foreground'
                                : 'text-muted-foreground hover:text-foreground'}"
                            onclick={() => (theme.mode = m.id)}
                        >
                            {m.label}
                        </button>
                    {/each}
                </div>

                <!-- Theme palette -->
                <p
                    class="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground"
                >
                    Theme
                </p>
                <div class="flex flex-col gap-1">
                    {#each PALETTES as p (p.id)}
                        <button
                            type="button"
                            class="flex items-center gap-2 rounded-md px-2 py-2.5 text-left text-sm hover:bg-muted {theme.palette ===
                            p.id
                                ? 'font-medium text-foreground'
                                : 'text-muted-foreground'}"
                            onclick={() => (theme.palette = p.id)}
                        >
                            <Check
                                class="h-4 w-4 shrink-0 {theme.palette === p.id
                                    ? 'opacity-100'
                                    : 'opacity-0'}"
                            />
                            <span>{p.name}</span>
                        </button>
                    {/each}
                </div>

                <!-- Font -->
                <p
                    class="mb-2 mt-4 text-xs font-semibold uppercase tracking-wide text-muted-foreground"
                >
                    Font
                </p>
                <div class="flex flex-col gap-1">
                    {#each FONTS as f (f.id)}
                        <button
                            type="button"
                            class="flex items-center gap-2 rounded-md px-2 py-2.5 text-left text-sm hover:bg-muted {theme.font ===
                            f.id
                                ? 'font-medium text-foreground'
                                : 'text-muted-foreground'}"
                            onclick={() => (theme.font = f.id)}
                        >
                            <Check
                                class="h-4 w-4 shrink-0 {theme.font === f.id
                                    ? 'opacity-100'
                                    : 'opacity-0'}"
                            />
                            <span style="font-family: {f.stack}">{f.name}</span>
                        </button>
                    {/each}
                </div>
            </div>
        </div>
    </div>
{/if}

<!-- Context menu -->
{#if menu}
    {@const node = menu.node}
    <div
        use:portal
        class="fixed z-50 min-w-36 rounded-md border border-border bg-popover py-1 text-sm shadow-lg"
        style="left: {menu.x}px; top: {menu.y}px"
        role="menu"
        tabindex="-1"
    >
        {#if node.is_dir}
            <button
                class="block w-full px-3 py-1 text-left hover:bg-muted"
                onclick={() => act("new-note", node)}>New note</button
            >
            <button
                class="block w-full px-3 py-1 text-left hover:bg-muted"
                onclick={() => act("new-folder", node)}>New folder</button
            >
            <!-- Notes have no rename: their name follows the first H1. Folders still do. -->
            <button
                class="block w-full px-3 py-1 text-left hover:bg-muted"
                onclick={() => act("rename", node)}>Rename</button
            >
        {/if}
        <button
            class="block w-full px-3 py-1 text-left text-destructive hover:bg-muted"
            onclick={() => act("delete", node)}>Delete</button
        >
    </div>
{/if}

<!-- Dialog (replaces window.prompt/confirm/alert) -->
{#if dialog}
    {@const d = dialog}
    <div
        use:portal
        class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
        role="presentation"
        onclick={(e) => {
            if (e.target === e.currentTarget)
                resolveDialog(d.kind === "confirm" ? false : null);
        }}
    >
        <div
            class="w-full max-w-sm rounded-lg border border-border bg-popover p-4 shadow-xl"
        >
            <p class="mb-3 text-sm font-medium wrap-break-word">
                {d.kind === "join" ? "Join vault" : d.title}
            </p>

            {#if d.kind === "join"}
                <input
                    use:focusSelect
                    class="mb-3 w-full rounded border border-border bg-background px-2 py-1.5 text-sm outline-none focus:border-primary"
                    placeholder="Paste vault ticket"
                    bind:value={d.value}
                    onkeydown={(e) => {
                        if (e.key === "Enter") resolveDialog(d.value);
                        else if (e.key === "Escape") resolveDialog(null);
                    }}
                />
                <div class="flex items-center justify-between gap-2">
                    {#if canScan}
                        <button
                            class="flex items-center gap-1.5 rounded border border-border px-3 py-1.5 text-sm hover:bg-muted"
                            onclick={async () => {
                                const c = await scanQr();
                                if (c) resolveDialog(c);
                            }}
                        >
                            <ScanLine class="h-4 w-4" /> Scan QR
                        </button>
                    {:else}
                        <span></span>
                    {/if}
                    <div class="flex gap-2">
                        <button
                            class="rounded px-3 py-1.5 text-sm text-muted-foreground hover:bg-muted"
                            onclick={() => resolveDialog(null)}>Cancel</button
                        >
                        <button
                            class="rounded bg-primary px-3 py-1.5 text-sm text-primary-foreground hover:bg-primary/90"
                            onclick={() => resolveDialog(d.value)}>Join</button
                        >
                    </div>
                </div>
            {:else if d.kind === "text"}
                <input
                    use:focusSelect
                    class="mb-3 w-full rounded border border-border bg-background px-2 py-1.5 text-sm outline-none focus:border-primary"
                    bind:value={d.value}
                    onkeydown={(e) => {
                        if (e.key === "Enter") resolveDialog(d.value);
                        else if (e.key === "Escape") resolveDialog(null);
                    }}
                />
                <div class="flex justify-end gap-2">
                    <button
                        class="rounded px-3 py-1 text-sm text-muted-foreground hover:bg-muted"
                        onclick={() => resolveDialog(null)}>Cancel</button
                    >
                    <button
                        class="rounded bg-primary px-3 py-1 text-sm text-primary-foreground hover:bg-primary/90"
                        onclick={() => resolveDialog(d.value)}>OK</button
                    >
                </div>
            {:else if d.kind === "confirm"}
                <div class="flex justify-end gap-2">
                    <button
                        class="rounded px-3 py-1 text-sm text-muted-foreground hover:bg-muted"
                        onclick={() => resolveDialog(false)}>Cancel</button
                    >
                    <button
                        class="rounded bg-destructive px-3 py-1 text-sm text-white hover:bg-destructive/90"
                        onclick={() => resolveDialog(true)}>OK</button
                    >
                </div>
            {:else}
                {#if d.qr}
                    <img
                        src={d.qr}
                        alt="Vault ticket QR code"
                        class="mx-auto mb-3 rounded bg-white p-2"
                        width="220"
                        height="220"
                    />
                {/if}
                <textarea
                    class="mb-3 h-28 w-full resize-none rounded border border-border bg-background px-2 py-1.5 font-mono text-xs outline-none"
                    readonly
                    onclick={(e) =>
                        (e.currentTarget as HTMLTextAreaElement).select()}
                    >{d.value}</textarea
                >
                <p class="mb-3 text-xs text-muted-foreground">
                    Scan the QR or copy the ticket. Either grants write access.
                </p>
                <div class="flex justify-end gap-2">
                    <button
                        class="rounded px-3 py-1 text-sm text-muted-foreground hover:bg-muted"
                        onclick={() =>
                            navigator.clipboard
                                ?.writeText(d.value)
                                .catch(() => {})}>Copy</button
                    >
                    <button
                        class="rounded bg-primary px-3 py-1 text-sm text-primary-foreground hover:bg-primary/90"
                        onclick={() => resolveDialog(null)}>Done</button
                    >
                </div>
            {/if}
        </div>
    </div>
{/if}
