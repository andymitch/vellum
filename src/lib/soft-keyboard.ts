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

/// How much shorter the viewport has to get before it counts as a keyboard
/// rather than a scrollbar, a URL bar, or a stray pixel.
const KEYBOARD_MIN_PX = 150;

export interface SoftKeyboard {
  /// Feed a viewport sample; returns whether the keyboard now reads as up.
  /// `layoutWidth`/`layoutHeight` are the *layout* viewport
  /// (`window.innerWidth` / `documentElement.clientHeight`),
  /// `viewportHeight` the visual one (`visualViewport.height`).
  sample(layoutWidth: number, viewportHeight: number, layoutHeight: number): boolean;
}

/// Whether the soft keyboard is up, inferred from the visual viewport.
///
/// The keyboard shrinks the visual viewport in both adjustResize and adjustPan
/// modes, so "shorter than the tallest we have seen" is the signal. What that
/// misses is that the viewport gets shorter for other reasons too, and the
/// tallest height only means something within one layout:
///
/// - **Rotation.** Landscape is shorter than portrait, so a baseline carried
///   over from portrait reads as a keyboard that is up and never comes down —
///   the toolbar pinned open, and anything waiting for a dismissal waiting
///   forever. Layout *width* tells the two apart: rotating changes it, a
///   keyboard does not. A new width starts the baseline again.
/// - **Launching with the keyboard already up.** There is no taller height on
///   record yet, so the keyboard reads as down and its first dismissal is
///   missed. Under adjustPan the layout viewport keeps its full height while
///   the keyboard is up, so taking that into the baseline recovers the real
///   height; under adjustResize it shrinks too and this is no worse.
export function softKeyboard(): SoftKeyboard {
  let baseWidth: number | null = null;
  let tallest = 0;
  return {
    sample(layoutWidth: number, viewportHeight: number, layoutHeight: number): boolean {
      if (layoutWidth !== baseWidth) {
        baseWidth = layoutWidth;
        tallest = 0;
      }
      tallest = Math.max(tallest, viewportHeight, layoutHeight);
      return viewportHeight < tallest - KEYBOARD_MIN_PX;
    },
  };
}

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
