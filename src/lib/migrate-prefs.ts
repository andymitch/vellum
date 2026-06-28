// One-time localStorage key migration: the project was renamed from "notes" to
// "vellum" (#78), which renamed every preference key. Copy the old `notes-*`
// value to its `vellum-*` counterpart on first launch so an upgrading user keeps
// their theme / session / editor settings / onboarding state instead of getting
// reset to defaults.
//
// Runs at MODULE-EVAL time (top level), and is imported FIRST in main.ts — the
// preference stores (theme.svelte.ts etc.) read localStorage at their own
// module-eval, so this must complete before those modules evaluate.
//
// The old keys are left in place for now (harmless; covers users who skip a
// release and downgrade). Drop this shim and the leftover `notes-*` keys after
// a release or two.

const RENAMES: [string, string][] = [
  ["notes-theme", "vellum-theme"],
  ["notes-monet", "vellum-monet"],
  ["notes-session", "vellum-session"],
  ["notes-editor", "vellum-editor"],
  ["notes-live-sync", "vellum-live-sync"],
  ["notes-seeded", "vellum-seeded"],
];

try {
  for (const [oldKey, newKey] of RENAMES) {
    const old = localStorage.getItem(oldKey);
    if (old !== null && localStorage.getItem(newKey) === null) {
      localStorage.setItem(newKey, old);
    }
  }
} catch {
  /* private mode / no storage — defaults are fine */
}
