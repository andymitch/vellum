// Svelte action: reparent an element to <body>. A drawer's translate-x transform
// makes its container a containing block for `position: fixed`, which would trap
// overlays (sheets, menus) inside the drawer instead of filling the viewport.
export function portal(node: HTMLElement) {
  document.body.appendChild(node);
  return { destroy: () => node.remove() };
}
