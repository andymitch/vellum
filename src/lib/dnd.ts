// Shared drag payload for the file-tree drag-and-drop. WebKit (the Tauri
// webview) strips custom dataTransfer MIME types, so for same-app drags we
// stash the dragged node here on dragstart and read it on drop.
export const drag: { item: { path: string; is_dir: boolean } | null } = { item: null };
