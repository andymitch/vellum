<script lang="ts">
    import { onMount } from "svelte";
    import {
        listVaults,
        createVault,
        joinVault,
        shareVault,
        forgetVault,
        listTree,
        readNote,
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
        LoaderCircle,
    } from "@lucide/svelte";
    import QRCode from "qrcode";
    import {
        scan,
        checkPermissions,
        requestPermissions,
    } from "@tauri-apps/plugin-barcode-scanner";
    import { session } from "$lib/session.svelte";
    import { portal } from "$lib/portal";
    import { drag } from "$lib/dnd";
    import { createAndOpenNote, duplicateNote } from "$lib/notes";

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
        ontree,
    }: {
        activePath?: string | null;
        onopen: (vault: string, path: string, focus?: boolean) => void;
        onvaultchange: (vault: string | null) => void;
        // Notify the parent of the current tree (used to derive the folder list
        // for move/duplicate actions).
        ontree?: (tree: TreeNode[]) => void;
    } = $props();

    // True until we've restored the saved vault + note once on launch, so manual
    // vault switches afterward don't reopen the persisted note.
    let restoring = true;

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

    const activeVaultName = $derived.by(() => {
        const v = vaults.find((v) => v.id === activeVault);
        if (v?.pending) return "Waiting for a peer…";
        return v?.name ?? (vaults.length ? "Select vault" : "No vaults");
    });

    // In-app dialogs. wry only implements window.prompt/confirm/alert on Android
    // (no macOS WKWebView impl), so those silently no-op on desktop. Use our own.
    type Dialog =
        | {
              kind: "text";
              title: string;
              value: string;
              // Hold the soft keyboard open across the post-resolve async work
              // (create + open) by handing focus to the keyboard-keeper input on
              // submit, so a new note's editor inherits an already-open keyboard.
              keep?: boolean;
              resolve: (v: string | null) => void;
          }
        | { kind: "confirm"; title: string; resolve: (v: boolean) => void }
        | { kind: "join"; value: string; resolve: (v: string | null) => void }
        | {
              kind: "move";
              title: string;
              choices: { path: string; label: string }[];
              resolve: (dir: string | null) => void;
          }
        | {
              kind: "share";
              title: string;
              value: string;
              qr: string;
              resolve: () => void;
          };
    let dialog = $state<Dialog | null>(null);
    // Off-screen input that holds the soft keyboard open between a name dialog's
    // submit and the new note's editor mounting (see resolveDialog).
    let kbKeeper: HTMLInputElement | undefined;

    const askText = (title: string, value = "", keep = false): Promise<string | null> =>
        new Promise(
            (resolve) => (dialog = { kind: "text", title, value, keep, resolve }),
        );
    const askJoin = (): Promise<string | null> =>
        new Promise(
            (resolve) => (dialog = { kind: "join", value: "", resolve }),
        );
    const askConfirm = (title: string): Promise<boolean> =>
        new Promise(
            (resolve) => (dialog = { kind: "confirm", title, resolve }),
        );
    // Folder targets a node can move into: every folder except its current
    // parent (a no-op) and — for a folder — itself or any descendant. Root is
    // offered when the node isn't already at root.
    function folderChoices(node: TreeNode): { path: string; label: string }[] {
        const cur = dirOf(node.path);
        const out: { path: string; label: string }[] = [];
        if (cur !== "") out.push({ path: "", label: "/ (root)" });
        const walkDirs = (nodes: TreeNode[]) => {
            for (const n of nodes) {
                if (!n.is_dir) continue;
                const selfOrDesc =
                    node.is_dir && (n.path === node.path || n.path.startsWith(node.path + "/"));
                if (n.path !== cur && !selfOrDesc) out.push({ path: n.path, label: n.path });
                walkDirs(n.children);
            }
        };
        walkDirs(tree);
        return out;
    }
    const askMove = (node: TreeNode): Promise<string | null> =>
        new Promise(
            (resolve) =>
                (dialog = {
                    kind: "move",
                    title: `Move "${displayName(node)}" to…`,
                    choices: folderChoices(node),
                    resolve,
                }),
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

    function resolveDialog(result: string | boolean | null) {
        const d = dialog;
        // Before tearing the dialog input out of the DOM (which would blur it and
        // let the soft keyboard start closing), hand focus to the off-screen
        // keyboard-keeper. This runs inside the submit gesture, so the keyboard
        // stays up through the async create/open that follows; the new note's
        // editor then inherits it. Without this, Android drops the keyboard and
        // the later editor.focus() can't reopen it (outside user activation).
        if (d?.kind === "text" && d.keep && result && kbKeeper)
            kbKeeper.focus({ preventScroll: true });
        dialog = null;
        if (!d) return;
        if (d.kind === "text" || d.kind === "join" || d.kind === "move")
            d.resolve(result as string | null);
        else if (d.kind === "confirm") d.resolve(result as boolean);
        else d.resolve();
    }

    const dirOf = (path: string) => path.split("/").slice(0, -1).join("/");
    const join = (dir: string, name: string) => (dir ? `${dir}/${name}` : name);
    // User-facing name: notes hide their .md extension (matches the tree). The
    // .md is re-applied on rename.
    const displayName = (n: TreeNode) => (n.is_dir ? n.name : n.name.replace(/\.md$/, ""));

    async function refreshVaults() {
        vaults = await listVaults();
    }
    async function refreshTree() {
        tree = activeVault ? await listTree(activeVault) : [];
        ontree?.(tree);
    }

    function treeHasFile(nodes: TreeNode[], path: string): boolean {
        for (const n of nodes) {
            if (!n.is_dir && n.path === path) return true;
            if (n.children && treeHasFile(n.children, path)) return true;
        }
        return false;
    }

    async function setActive(id: string | null) {
        // Capture the saved note before onvaultchange clears it (restore only).
        const wantPath = restoring ? session.path : null;
        activeVault = id;
        expanded = {};
        onvaultchange(id);
        session.vault = id;
        if (id) {
            await watchVault(id);
            await refreshTree();
            // On launch, reopen the saved note if it still exists.
            if (wantPath && treeHasFile(tree, wantPath)) onopen(id, wantPath);
        } else {
            tree = [];
            ontree?.([]);
        }
        restoring = false;
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

    // Create a note in `dir` ("" = vault root). Filename and content are
    // independent, so we prompt for a name up front (createNote de-dupes).
    async function newNoteIn(vault: string, dir: string) {
        // keep=true: hold the keyboard open so the new note's editor gets focus
        // with the keyboard already up (#50).
        const name = (await askText("New note name", "", true))?.trim();
        if (!name) return;
        if (dir) expanded[dir] = true;
        await createAndOpenNote(vault, dir, name, onopen);
        await refreshTree();
    }

    async function newRootNote() {
        if (!activeVault) return;
        await newNoteIn(activeVault, "");
    }


    // One-tap new note (#85): create + open "Untitled" in `dir` with no name
    // prompt — createNote de-dupes to "Untitled 1", "Untitled 2", … The user
    // starts typing immediately and renames later (breadcrumb / settings) if
    // they want.
    export async function newUntitledNote(dir = "") {
        if (!activeVault) return;
        if (dir) expanded[dir] = true;
        await createAndOpenNote(activeVault, dir, "Untitled", onopen);
        await refreshTree();
    }

    // Rename the currently-open note. Used by the settings sheet and the
    // breadcrumb (double-click / long-press) so all rename entry points share
    // the same prompt + path logic.
    export function renameActive() {
        if (!activeVault || !activePath) return;
        const name = activePath.split("/").pop() ?? "";
        act("rename", { name, path: activePath, is_dir: false, children: [] });
    }

    async function newRootFolder() {
        if (!activeVault) return;
        const name = (await askText("New folder name"))?.trim();
        if (!name) return;
        await createFolder(activeVault, name);
        await refreshTree();
    }

    // Imperative hooks for global hotkeys (App owns the keydown listener).
    export function createFolderHotkey() {
        newRootFolder();
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
            // Prefill/compare without the .md (it's hidden from the user); re-add it below.
            const current = displayName(node);
            const name = (await askText("Rename to", current))?.trim();
            if (!name || name === current) return;
            // Keep the .md extension on notes even if the user drops it.
            const final = node.is_dir || name.endsWith(".md") ? name : `${name}.md`;
            const to = join(dirOf(node.path), final);
            await renamePath(activeVault, node.path, to, node.is_dir);
            if (activePath === node.path) onopen(activeVault, to);
        } else if (kind === "move") {
            const dir = await askMove(node);
            if (dir === null) return;
            await moveTo(node.path, node.is_dir, dir); // refreshes the tree itself
            return;
        } else if (kind === "duplicate") {
            const finalPath = await duplicateNote(activeVault, node.path, tree);
            onopen(activeVault, finalPath);
        } else if (kind === "copy") {
            try {
                await navigator.clipboard.writeText(await readNote(activeVault, node.path));
            } catch {
                /* clipboard may be unavailable */
            }
            return; // nothing changed on disk
        } else if (kind === "delete") {
            if (!(await askConfirm(`Delete "${displayName(node)}"?`))) return;
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

    // Drag-and-drop move (desktop): move `from` into folder `toDir` ("" = root).
    // Follows the open note if it (or its containing folder) moved.
    const dndEnabled = !window.matchMedia("(max-width: 767px)").matches;
    let rootDragOver = $state(false);
    async function moveTo(from: string, isDir: boolean, toDir: string) {
        if (!activeVault || !from) return;
        // No-op if already there; never drop a folder into itself or a descendant.
        if (dirOf(from) === toDir) return;
        if (toDir === from || toDir.startsWith(from + "/")) return;
        const dest = join(toDir, from.split("/").pop()!);
        await renamePath(activeVault, from, dest, isDir);
        if (activePath === from) onopen(activeVault, dest);
        else if (activePath && activePath.startsWith(from + "/"))
            onopen(activeVault, dest + activePath.slice(from.length));
        await refreshTree();
    }

    const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

    // First-run starter pack (#51): seed a "My Vault" with a welcome note so a
    // brand-new install isn't an empty void. Gated by a localStorage flag so it
    // runs once — deleting everything later won't regenerate it.
    const SEEDED_KEY = "notes-seeded";
    const WELCOME_NOTE = `# Hello Vellum 👋

Welcome to **Vellum** — a local-first Markdown notes app that syncs **peer-to-peer**. No account, no server, no cloud; your notes live on your devices.

## The basics
- Notes are **Markdown**. Toggle **source ⟷ preview** with the buttons in the top bar.
- Organize notes into folders inside a **vault** — make as many vaults as you like.
- Everything saves automatically as you type.

## Sync across devices
Tap **Share** on a vault to show a QR code, then **Join** it from another device. Edits flow both ways automatically — over the internet or the same Wi-Fi.

## Markdown quick reference
- \`**bold**\`, \`*italic*\`, \`\\\`code\\\`\`
- \`- [ ]\` task list
- \`> quote\`, \`# headings\`

Delete this note whenever you're ready. Happy writing!
`;

    async function seedStarterPack() {
        const v = await createVault("My Vault");
        const path = await createNote(v.id, "Hello Vellum.md");
        await writeNote(v.id, path, WELCOME_NOTE);
        await refreshVaults();
        await setActive(v.id);
        onopen(v.id, path);
    }

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
            // First run with no vaults: seed the starter pack once (#51).
            if (!vaults.length && !localStorage.getItem(SEEDED_KEY)) {
                localStorage.setItem(SEEDED_KEY, "1");
                try {
                    await seedStarterPack();
                } catch {
                    /* seeding is best-effort; don't block startup */
                }
            } else {
                // Restore the last-used vault if it still exists, else the first.
                const want =
                    (session.vault && vaults.some((v) => v.id === session.vault)
                        ? session.vault
                        : vaults[0]?.id) ?? null;
                if (want) await setActive(want);
            }
            restoring = false;
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

<!-- Keyboard-keeper: focused on a name-dialog submit to hold the soft keyboard
     open until the new note's editor mounts (see resolveDialog / #50). Off-screen
     and out of the tab order; never receives real input. -->
<input
    bind:this={kbKeeper}
    tabindex="-1"
    aria-hidden="true"
    autocapitalize="off"
    autocorrect="off"
    autocomplete="off"
    spellcheck="false"
    style="position:fixed;top:0;left:0;width:1px;height:1px;opacity:0;pointer-events:none;border:0;padding:0;"
/>

<div class="flex h-full flex-col">
    <!-- Vault selector + new note/folder -->
    <div class="flex items-center gap-1 px-2 py-2 md:py-1.5">
        <button
            type="button"
            class="flex min-w-0 flex-1 items-center gap-2 rounded px-2 py-1.5 text-left text-sm font-medium hover:bg-muted"
            onclick={() => (vaultSheet = true)}
        >
            <ChevronsUpDown class="h-4 w-4 shrink-0 text-muted-foreground" />
            <span class="truncate">{activeVaultName}</span>
        </button>
        <button
            type="button"
            class="shrink-0 rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground disabled:opacity-40 md:p-1"
            title="New note"
            disabled={!activeVault}
            onclick={newRootNote}
        >
            <FilePlus class="h-4.5 w-4.5 md:h-3.75 md:w-3.75" />
        </button>
        <button
            type="button"
            class="shrink-0 rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground disabled:opacity-40 md:p-1"
            title="New folder"
            disabled={!activeVault}
            onclick={newRootFolder}
        >
            <FolderPlus class="h-4.5 w-4.5 md:h-3.75 md:w-3.75" />
        </button>
    </div>

    <!-- Tree. The scroll area is a drop target for moving items to the vault root. -->
    <div
        class="min-h-0 flex-1 overflow-auto px-1 pb-2 {rootDragOver
            ? 'rounded bg-muted'
            : ''}"
        role="tree"
        tabindex="-1"
        ondragenter={dndEnabled ? (e) => e.preventDefault() : undefined}
        ondragover={dndEnabled
            ? (e) => {
                  e.preventDefault();
                  if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
                  rootDragOver = true;
              }
            : undefined}
        ondragleave={dndEnabled ? () => (rootDragOver = false) : undefined}
        ondrop={dndEnabled
            ? (e) => {
                  e.preventDefault();
                  rootDragOver = false;
                  const d = drag.item;
                  drag.item = null;
                  if (d) moveTo(d.path, d.is_dir, "");
              }
            : undefined}
    >
        {#if activeVault && tree.length}
            <Tree
                nodes={tree}
                {activePath}
                {expanded}
                dnd={dndEnabled}
                onselect={(node) => onopen(activeVault!, node.path)}
                onmenu={openMenu}
                onmove={moveTo}
            />
        {:else if activeVault}
            <p class="px-2 py-4 text-sm text-muted-foreground">Empty vault.</p>
        {:else}
            <p class="px-2 py-4 text-sm text-muted-foreground">
                Create or join a vault to begin.
            </p>
        {/if}
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
                            {#if v.pending}
                                <span class="flex min-w-0 flex-col">
                                    <span class="truncate italic">{v.name}</span>
                                    <span
                                        class="flex items-center gap-1 text-xs text-muted-foreground"
                                    >
                                        <LoaderCircle class="h-3 w-3 shrink-0 animate-spin" />
                                        Waiting for a peer…
                                    </span>
                                </span>
                            {:else}
                                <span class="truncate">{v.name}</span>
                            {/if}
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
            <button
                class="block w-full px-3 py-1 text-left hover:bg-muted"
                onclick={() => act("rename", node)}>Rename</button
            >
        {:else}
            <button
                class="block w-full px-3 py-1 text-left hover:bg-muted"
                onclick={() => act("rename", node)}>Rename</button
            >
            <button
                class="block w-full px-3 py-1 text-left hover:bg-muted"
                onclick={() => act("move", node)}>Move to…</button
            >
            <button
                class="block w-full px-3 py-1 text-left hover:bg-muted"
                onclick={() => act("duplicate", node)}>Duplicate</button
            >
            <button
                class="block w-full px-3 py-1 text-left hover:bg-muted"
                onclick={() => act("copy", node)}>Copy contents</button
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
                    autocapitalize="off"
                    autocorrect="off"
                    autocomplete="off"
                    spellcheck="false"
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
                    autocapitalize="off"
                    autocorrect="off"
                    autocomplete="off"
                    spellcheck="false"
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
            {:else if d.kind === "move"}
                {#if d.choices.length}
                    <div class="mb-3 max-h-60 overflow-auto">
                        {#each d.choices as c (c.path)}
                            <button
                                class="block w-full truncate rounded px-2 py-1.5 text-left text-sm hover:bg-muted"
                                onclick={() => resolveDialog(c.path)}>{c.label}</button
                            >
                        {/each}
                    </div>
                {:else}
                    <p class="mb-3 text-sm text-muted-foreground">
                        No other folders to move into.
                    </p>
                {/if}
                <div class="flex justify-end">
                    <button
                        class="rounded px-3 py-1 text-sm text-muted-foreground hover:bg-muted"
                        onclick={() => resolveDialog(null)}>Cancel</button
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
                    spellcheck="false"
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
