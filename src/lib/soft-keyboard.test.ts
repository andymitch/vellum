// The keyboard-dismissal rule, which is what finishes an edit on mobile.
//
// Worth its own test because both of its subtleties are invisible in the call
// site: a dismissal has to have been preceded by the keyboard actually being
// up, and it has to fire exactly once — a repeated `false` (the viewport
// settling, an unrelated resize) is not a second dismissal.
/// <reference types="bun-types" />
import { describe, expect, test } from "bun:test";
import { keyboardDismissal, softKeyboard } from "./soft-keyboard";

// A phone in portrait, then the same phone in landscape. Heights are the
// visual viewport; the layout viewport matches it unless the keyboard is up
// under adjustPan, which is the third argument's whole reason for existing.
const PORTRAIT = { w: 412, h: 869 };
const LANDSCAPE = { w: 869, h: 412 };
const KEYBOARD = 320; // roughly what a soft keyboard takes

describe("softKeyboard", () => {
  test("reads the keyboard opening and closing", () => {
    const kb = softKeyboard();
    expect(kb.sample(PORTRAIT.w, PORTRAIT.h, PORTRAIT.h)).toBe(false);
    const shrunk = PORTRAIT.h - KEYBOARD;
    expect(kb.sample(PORTRAIT.w, shrunk, shrunk)).toBe(true);
    expect(kb.sample(PORTRAIT.w, PORTRAIT.h, PORTRAIT.h)).toBe(false);
  });

  test("rotating to landscape is not a keyboard", () => {
    // The bug this fixes: landscape is shorter than portrait, so a baseline
    // carried over from portrait read as a keyboard that never came down.
    const kb = softKeyboard();
    kb.sample(PORTRAIT.w, PORTRAIT.h, PORTRAIT.h);
    expect(kb.sample(LANDSCAPE.w, LANDSCAPE.h, LANDSCAPE.h)).toBe(false);
  });

  test("and the keyboard still reads in landscape afterwards", () => {
    const kb = softKeyboard();
    kb.sample(PORTRAIT.w, PORTRAIT.h, PORTRAIT.h);
    kb.sample(LANDSCAPE.w, LANDSCAPE.h, LANDSCAPE.h);
    const shrunk = LANDSCAPE.h - KEYBOARD;
    expect(kb.sample(LANDSCAPE.w, shrunk, shrunk)).toBe(true);
    expect(kb.sample(LANDSCAPE.w, LANDSCAPE.h, LANDSCAPE.h)).toBe(false);
  });

  test("rotating back to portrait is not a keyboard either", () => {
    const kb = softKeyboard();
    kb.sample(PORTRAIT.w, PORTRAIT.h, PORTRAIT.h);
    kb.sample(LANDSCAPE.w, LANDSCAPE.h, LANDSCAPE.h);
    expect(kb.sample(PORTRAIT.w, PORTRAIT.h, PORTRAIT.h)).toBe(false);
  });

  test("a keyboard already up at launch is seen, when the layout says so", () => {
    // adjustPan: the layout viewport keeps its full height, so the first
    // sample has something to compare against and the keyboard is not missed.
    const kb = softKeyboard();
    expect(kb.sample(PORTRAIT.w, PORTRAIT.h - KEYBOARD, PORTRAIT.h)).toBe(true);
    expect(kb.sample(PORTRAIT.w, PORTRAIT.h, PORTRAIT.h)).toBe(false);
  });

  test("a small shrink is not a keyboard", () => {
    const kb = softKeyboard();
    kb.sample(PORTRAIT.w, PORTRAIT.h, PORTRAIT.h);
    // A URL bar or a stray pixel — under the threshold.
    expect(kb.sample(PORTRAIT.w, PORTRAIT.h - 100, PORTRAIT.h - 100)).toBe(false);
  });
});

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
