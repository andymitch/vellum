// Which scrolls are the app's own doing.
//
// The mobile chrome hides on scroll-down and returns on scroll-up (#85), and it
// decides that from the change in scrollTop — which cannot by itself tell a
// finger from the app moving the note under one. Reading our own scrolling as
// the user's slid the header away mid-sentence (#248).
//
// The app scrolls the note in a handful of places: restoring a position, pinning
// the caret under a quick-edit tap, holding it there while the keyboard opens,
// and jumping to an internal link or heading. Each marks the scroll it is about
// to make, and a marked scroll never moves the chrome. This lives in its own
// module because those places are spread across App, Preview and Editor.

let until = 0;

/// Claim the scroll events about to arrive as the app's own. `ms` needs to
/// outlast them: a scrollTop assignment lands its event in the same frame, but a
/// `behavior: "smooth"` scroll keeps emitting them for a few hundred.
export function markAppScroll(ms = 120): void {
  const deadline = Date.now() + ms;
  // Never shorten a claim already in flight — a short mark landing inside a
  // smooth scroll would hand the rest of it back to the chrome.
  if (deadline > until) until = deadline;
}

/// Whether a scroll arriving right now is one we claimed.
export const appScrolling = (): boolean => Date.now() < until;
