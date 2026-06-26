// Generate categorized release notes for a tag range.
//
// For each PR merged since the previous release, the category is taken from the
// label of the issue it closes (so you only label issues, not PRs); if the PR
// closes no labeled issue, it falls back to the PR's own labels, else "Other".
//
// Env: REPO (owner/name), TAG (e.g. v3), PREV (previous tag, or empty for the
// first release). Auth via the gh CLI. Prints markdown to stdout.
import { execFileSync } from "node:child_process";

const { REPO, TAG, PREV } = process.env;
const [OWNER, NAME] = REPO.split("/");
// execFile (no shell) — args are passed literally, so nothing is shell-interpreted.
const ghJson = (args) =>
  JSON.parse(
    // stderr ignored: a #N in a commit message may be an issue, not a PR — the
    // lookup throws and we skip it, no need to surface gh's error.
    execFileSync("gh", args, { encoding: "utf8", maxBuffer: 1 << 24, stdio: ["ignore", "pipe", "ignore"] }),
  );

// Label -> section. First match wins; order defines section order in the notes.
const CATEGORIES = [
  { title: "✨ Features", labels: ["feature", "enhancement"] },
  { title: "🐛 Fixes", labels: ["bug"] },
  { title: "🔧 Maintenance", labels: ["chore", "documentation", "dependencies"] },
];
const OTHER = "Other";
const categoryFor = (labels) =>
  CATEGORIES.find((c) => labels.some((l) => c.labels.includes(l)))?.title ?? OTHER;

// Collect PR numbers merged in PREV..TAG (or all history for the first release)
// by parsing commit messages — robust to history rewrites (which break GitHub's
// commit↔PR association). Matches merge commits ("Merge pull request #N") and
// squash commits ("… (#N)" on the first line).
const messages = PREV
  ? ghJson(["api", `repos/${REPO}/compare/${PREV}...${TAG}`, "--jq", "[.commits[].commit.message]"])
  : ghJson(["api", `repos/${REPO}/commits`, "--paginate", "--jq", "[.[].commit.message]"]);

const prNums = new Set();
for (const msg of messages) {
  const firstLine = msg.split("\n")[0];
  const m = firstLine.match(/Merge pull request #(\d+)/) || firstLine.match(/\(#(\d+)\)\s*$/);
  if (m) prNums.add(Number(m[1]));
}

const sections = {};
for (const n of [...prNums].sort((a, b) => a - b)) {
  const query = `query{repository(owner:"${OWNER}",name:"${NAME}"){pullRequest(number:${n}){title author{login} labels(first:20){nodes{name}} closingIssuesReferences(first:10){nodes{labels(first:20){nodes{name}}}}}}}`;
  let pr;
  try {
    pr = ghJson(["api", "graphql", "-f", `query=${query}`]).data.repository.pullRequest;
  } catch {
    continue;
  }
  if (!pr) continue;
  const prLabels = pr.labels.nodes.map((x) => x.name);
  const issueLabels = pr.closingIssuesReferences.nodes.flatMap((i) =>
    i.labels.nodes.map((x) => x.name),
  );
  // Issue label first, then the PR's own label.
  let cat = categoryFor(issueLabels);
  if (cat === OTHER) cat = categoryFor(prLabels);
  const author = pr.author?.login ? ` @${pr.author.login}` : "";
  (sections[cat] ??= []).push(`- ${pr.title} (#${n})${author}`);
}

let md = "";
for (const cat of [...CATEGORIES.map((c) => c.title), OTHER]) {
  if (sections[cat]?.length) md += `### ${cat}\n${sections[cat].join("\n")}\n\n`;
}
md += `**Full Changelog**: https://github.com/${REPO}/commits/${TAG}\n`;
process.stdout.write(md);
