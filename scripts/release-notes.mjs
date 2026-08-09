// Generate categorized release notes for a tag range.
//
// For each PR merged since the previous release, the category is taken from the
// label of the issue it closes (so you only label issues, not PRs); if the PR
// closes no labeled issue, it falls back to the PR's own labels, else "Other".
//
// When cutting a STABLE release after a beta cycle, BUGS both raised and fixed
// inside that cycle are folded away (#182/#198): a bug a beta introduced never
// reached anyone on stable. Features and enhancements raised in-cycle are kept —
// they are new capability, and dropping them made the notes describe software
// that never shipped. See `cycleStart` below.
//
// Env: REPO (owner/name), TAG (e.g. v3), PREV (previous tag, or empty for the
// first release). Auth via the gh CLI. Prints markdown to stdout; notes about
// dropped entries go to stderr so a release never silently omits anything.
import { execFileSync } from "node:child_process";

// HEAD is the commit to diff up to. In CI the vN tag doesn't exist yet (the
// release is still a draft — publishing creates the tag), so a compare against
// TAG 404s; pass the release's target SHA as HEAD. Falls back to TAG locally.
const { REPO, TAG, PREV, HEAD } = process.env;
const [OWNER, NAME] = REPO.split("/");
// execFile (no shell) — args are passed literally, so nothing is shell-interpreted.
const ghJson = (args) =>
  JSON.parse(
    // stderr ignored: a #N in a commit message may be an issue, not a PR — the
    // lookup throws and we skip it, no need to surface gh's error.
    execFileSync("gh", args, { encoding: "utf8", maxBuffer: 1 << 24, stdio: ["ignore", "pipe", "ignore"] }),
  );

// An issue labelled this is kept even if it was raised during the beta cycle —
// the escape hatch for a genuinely pre-existing bug that merely happened to be
// DISCOVERED while testing a beta.
const FORCE_LABEL = "changelog-include";

// Start of the current beta cycle: when the earliest vN-beta.* for this major
// was published. Null when cutting a beta itself (a beta's notes should list
// everything since the last stable, so testers see the whole surface they are
// being asked to exercise) or when the release had no betas.
const cycleStart = (() => {
  if (/-beta\./.test(TAG)) return null;
  const major = /^v(\d+)/.exec(TAG)?.[1];
  if (!major) return null;
  let dates = [];
  try {
    dates = ghJson([
      "api",
      `repos/${REPO}/releases`,
      "--paginate",
      "--jq",
      `[.[] | select(.tag_name | startswith("v${major}-beta.")) | .published_at]`,
    ]);
  } catch {
    return null;
  }
  return dates.length ? dates.sort()[0] : null;
})();
if (cycleStart) process.stderr.write(`Beta cycle started ${cycleStart}; folding in-cycle work.\n`);

// Label -> section. First match wins; order defines section order in the notes.
const CATEGORIES = [
  { title: "✨ Features", labels: ["feature", "enhancement"] },
  { title: "🐛 Fixes", labels: ["bug"] },
  { title: "🔧 Maintenance", labels: ["chore", "documentation", "dependencies"] },
];
const OTHER = "Other";
const categoryFor = (labels) =>
  CATEGORIES.find((c) => labels.some((l) => c.labels.includes(l)))?.title ?? OTHER;

// Collect merged work in PREV..TAG (or all history for the first release) by
// parsing commit messages — robust to history rewrites (which break GitHub's
// commit↔PR association). Three merge shapes are recognized:
//   - "Merge pull request #N …"  → PR #N (GitHub PR)
//   - "… (#N)" on the first line → PR #N (squash merge)
//   - "Merge branch 'feat/N-…'"  → issue #N (local branch merge, no PR)
// The last one matters because branches are often merged locally, with no PR —
// those carry no PR number, so we fall back to the issue the branch name names.
const messages = PREV
  ? ghJson(["api", `repos/${REPO}/compare/${PREV}...${HEAD || TAG}`, "--jq", "[.commits[].commit.message]"])
  : ghJson(["api", `repos/${REPO}/commits`, "--paginate", "--jq", "[.[].commit.message]"]);

const prNums = new Set();
const issueNums = new Set();
for (const msg of messages) {
  const firstLine = msg.split("\n")[0];
  const pr = firstLine.match(/Merge pull request #(\d+)/) || firstLine.match(/\(#(\d+)\)\s*$/);
  if (pr) {
    prNums.add(Number(pr[1]));
    continue;
  }
  // Local branch merge, e.g. "Merge branch 'feat/16-internal-links'" — the
  // leading number is the issue the branch addresses.
  const br = firstLine.match(/Merge branch '[a-z]+\/(\d+)-/);
  if (br) issueNums.add(Number(br[1]));
}

const sections = {};
// Entries folded away by the beta-cycle rule, reported at the end.
const dropped = [];
// Issues a PR already closes — so the same work merged via both a PR and a
// later local branch merge isn't listed twice.
const coveredByPr = new Set();
for (const n of [...prNums].sort((a, b) => a - b)) {
  const query = `query{repository(owner:"${OWNER}",name:"${NAME}"){pullRequest(number:${n}){title createdAt author{login} labels(first:20){nodes{name}} closingIssuesReferences(first:10){nodes{number createdAt labels(first:20){nodes{name}}}}}}}`;
  let pr;
  try {
    pr = ghJson(["api", "graphql", "-f", `query=${query}`]).data.repository.pullRequest;
  } catch {
    continue;
  }
  if (!pr) continue;
  for (const i of pr.closingIssuesReferences.nodes) coveredByPr.add(i.number);
  const prLabels0 = pr.labels.nodes.map((x) => x.name);
  const issueLabels0 = pr.closingIssuesReferences.nodes.flatMap((i) =>
    i.labels.nodes.map((x) => x.name),
  );
  let cat0 = categoryFor(issueLabels0);
  if (cat0 === OTHER) cat0 = categoryFor(prLabels0);

  // Fold away BUGS raised entirely within the beta cycle: a bug a beta
  // introduced and a beta patched never reached a stable user, so it is noise.
  //
  // Features and enhancements raised in-cycle are KEPT, even though they are
  // also "new since the last beta" — they are new capability, not restatements.
  // Folding them produced a v8 changelog announcing a "Scratchpad" type that had
  // since been replaced, while never mentioning the Journal that replaced it or
  // the beta-updates toggle at all (#198).
  //
  // A PR closing no issue has nothing to date, so fall back to when the PR
  // itself was opened.
  if (cycleStart && cat0 === "🐛 Fixes") {
    const issues = pr.closingIssuesReferences.nodes;
    const forced = issues.some((i) => i.labels.nodes.some((l) => l.name === FORCE_LABEL));
    const raised = issues.length ? issues.map((i) => i.createdAt) : [pr.createdAt];
    if (!forced && raised.every((d) => d > cycleStart)) {
      dropped.push(`#${n} ${pr.title}`);
      continue;
    }
  }
  // Issue label first, then the PR's own label (computed above).
  const cat = cat0;
  const author = pr.author?.login ? ` @${pr.author.login}` : "";
  (sections[cat] ??= []).push(`- ${pr.title} (#${n})${author}`);
}

// Branch-merge issues: categorize and title straight from the issue.
for (const n of [...issueNums].sort((a, b) => a - b)) {
  if (coveredByPr.has(n)) continue;
  const query = `query{repository(owner:"${OWNER}",name:"${NAME}"){issue(number:${n}){title labels(first:20){nodes{name}}}}}`;
  let issue;
  try {
    issue = ghJson(["api", "graphql", "-f", `query=${query}`]).data.repository.issue;
  } catch {
    continue;
  }
  if (!issue) continue;
  const cat = categoryFor(issue.labels.nodes.map((x) => x.name));
  (sections[cat] ??= []).push(`- ${issue.title} (#${n})`);
}

let md = "";
for (const cat of [...CATEGORIES.map((c) => c.title), OTHER]) {
  if (sections[cat]?.length) md += `### ${cat}\n${sections[cat].join("\n")}\n\n`;
}
md += `**Full Changelog**: https://github.com/${REPO}/commits/${TAG}\n`;
if (dropped.length) {
  process.stderr.write(
    `Folded ${dropped.length} in-cycle entries (raised and resolved during the beta):\n` +
      dropped.map((d) => `  - ${d}`).join("\n") +
      `\nLabel an issue "${FORCE_LABEL}" to keep it.\n`,
  );
}
process.stdout.write(md);
