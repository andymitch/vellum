// Heading slugs. Preview assigns them as heading ids and App scrolls to them for
// `[[note#heading]]`, so a change here silently breaks cross-note anchors (#45).
/// <reference types="bun-types" />
import { expect, test } from "bun:test";
import { slugify } from "./slug";

test("matches GitHub-style heading slugs", () => {
  expect(slugify("Getting Started")).toBe("getting-started");
  expect(slugify("  What's new?  ")).toBe("whats-new");
  expect(slugify("A -- B")).toBe("a-b");
  expect(slugify("v2.0 notes")).toBe("v20-notes");
  expect(slugify("snake_case stays")).toBe("snake_case-stays");
});
