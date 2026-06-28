// GitHub-style heading slug: lowercase, drop punctuation, spaces to hyphens.
// Shared so Preview (assigning heading ids) and App (scrolling to a cross-note
// anchor) compute the same slug for `[[note#heading]]` links (#45).
export const slugify = (s: string): string =>
  s
    .toLowerCase()
    .trim()
    .replace(/[^\w\s-]/g, "")
    .replace(/\s+/g, "-")
    .replace(/-+/g, "-");
