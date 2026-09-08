// Dismissing the soft keyboard, as an event.
//
// On mobile, hiding the keyboard is how you finish what you were typing — a
// journal chunk (#258). It is not a blur: Android and iOS both leave the
// focused element focused when the keyboard goes away, so nothing fires and a
// field wired to `onblur` sits there open forever.
//
// App applies the same rule inline to end a markdown note's quick edit (#33),
// where it is tangled up with re-pinning the caret across the frames the
// keyboard takes to open. Worth folding into this once that can be re-tested
// on a device — the pinning is what #122/#147 were about.
//
// What the keyboard *does* change is the visual viewport, which shrinks while
// it is up. App watches that and passes the result down as `kbOpen`. Turning
// that flag into "the user just dismissed it" needs one piece of state, and
// the reason is worth stating: an edit can begin while the keyboard is still
// animating in, when `kbOpen` is momentarily false — so a dismissal only
// counts once the keyboard has actually been up.

export interface KeyboardDismissal {
  /// Feed the current keyboard state. True exactly once per up-then-down,
  /// i.e. on the frame the user dismissed it.
  dismissed(open: boolean): boolean;
  /// Forget that the keyboard was up — call when the edit it would finish is
  /// over, so the next one starts from a clean slate.
  reset(): void;
}

export function keyboardDismissal(): KeyboardDismissal {
  let wasOpen = false;
  return {
    dismissed(open: boolean): boolean {
      if (open) {
        wasOpen = true;
        return false;
      }
      if (!wasOpen) return false;
      wasOpen = false;
      return true;
    },
    reset() {
      wasOpen = false;
    },
  };
}
