// The keyboard-dismissal rule, which is what finishes an edit on mobile.
//
// Worth its own test because both of its subtleties are invisible in the call
// site: a dismissal has to have been preceded by the keyboard actually being
// up, and it has to fire exactly once — a repeated `false` (the viewport
// settling, an unrelated resize) is not a second dismissal.
/// <reference types="bun-types" />
import { describe, expect, test } from "bun:test";
import { keyboardDismissal } from "./soft-keyboard";

describe("keyboardDismissal", () => {
  test("fires on the transition from up to down", () => {
    const kb = keyboardDismissal();
    expect(kb.dismissed(true)).toBe(false);
    expect(kb.dismissed(false)).toBe(true);
  });

  test("a keyboard that was never up cannot be dismissed", () => {
    // An edit begun while the keyboard is still animating in reads as closed
    // for a frame or two. Firing there would close the chunk on the tap that
    // opened it.
    const kb = keyboardDismissal();
    expect(kb.dismissed(false)).toBe(false);
    expect(kb.dismissed(false)).toBe(false);
  });

  test("fires once, not for every frame the keyboard stays down", () => {
    const kb = keyboardDismissal();
    kb.dismissed(true);
    expect(kb.dismissed(false)).toBe(true);
    expect(kb.dismissed(false)).toBe(false);
    expect(kb.dismissed(false)).toBe(false);
  });

  test("stays armed while the keyboard is up", () => {
    const kb = keyboardDismissal();
    expect(kb.dismissed(true)).toBe(false);
    expect(kb.dismissed(true)).toBe(false);
    expect(kb.dismissed(false)).toBe(true);
  });

  test("re-arms for the next edit", () => {
    const kb = keyboardDismissal();
    kb.dismissed(true);
    expect(kb.dismissed(false)).toBe(true);
    kb.dismissed(true);
    expect(kb.dismissed(false)).toBe(true);
  });

  test("reset forgets that the keyboard was up", () => {
    // The edit ended some other way — Escape, Return, tapping another chunk.
    // The keyboard may still be up for the *next* chunk, and its eventual
    // dismissal belongs to that one.
    const kb = keyboardDismissal();
    kb.dismissed(true);
    kb.reset();
    expect(kb.dismissed(false)).toBe(false);
  });
});
