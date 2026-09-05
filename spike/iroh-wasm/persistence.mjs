// Does a browser vault survive a reload? (PERSISTENCE.md step 2.)
//
// Tab 1 opens a vault whose replica lives on OPFS, writes two notes, and is
// closed — which is what releases OPFS's exclusive lock, and models a reload.
// Tab 2 then opens the same file and must find those notes *with their content*
// and no peer involved: read off disk, not synced from anywhere.
//
// The replica comes from redb on an OPFS sync access handle; the content from a
// second redb database beside it, keyed by content hash (see src/opfs.rs).
//
// Sessions 2 and 3 go on to check the three things that make it a store rather
// than a demo: identical content is not stored twice, superseded versions are
// collected on the next boot, and a second tab on the same vault is refused
// with a sentence rather than an OPFS error, and the device keeps one identity
// across all three. Every session flushes before
// closing — without that, a write can still be inside the docs actor's 500 ms
// batch window when the tab goes away.
const base = "http://127.0.0.1:9333";
const { webSocketDebuggerUrl } = await (await fetch(`${base}/json/version`)).json();
const ws = new WebSocket(webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r));
let id = 0;
const pending = new Map();
const logs = [];
ws.addEventListener("message", (e) => {
  const m = JSON.parse(e.data);
  if (m.id && pending.has(m.id)) pending.get(m.id)(m);
  if (m.method === "Runtime.consoleAPICalled")
    logs.push(`[${m.params.type}] ${m.params.args.map((a) => a.value ?? a.description).join(" ")}`);
  if (m.method === "Runtime.exceptionThrown")
    logs.push(
      `[exception] ${m.params.exceptionDetails.exception?.description ?? m.params.exceptionDetails.text}`,
    );
});
const send = (method, params = {}, sessionId) =>
  new Promise((res) => {
    const n = ++id;
    pending.set(n, res);
    ws.send(JSON.stringify({ id: n, method, params, sessionId }));
  });

const URL_ = "http://127.0.0.1:9334/persist.html";

async function tab() {
  const { result } = await send("Target.createTarget", { url: URL_ });
  const sessionId = (await send("Target.attachToTarget", { targetId: result.targetId, flatten: true }))
    .result.sessionId;
  await send("Runtime.enable", {}, sessionId);
  const evaluate = async (expression) => {
    const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true }, sessionId);
    if (r.result?.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails).slice(0, 400));
    return r.result?.result?.value;
  };
  for (let i = 0; ; i++) {
    try {
      if ((await evaluate("typeof window.api")) === "object") break;
    } catch {
      /* context replaced mid-navigation */
    }
    if (i > 300) throw new Error("page never came up");
    await new Promise((r) => setTimeout(r, 100));
  }
  return { evaluate, close: () => send("Target.closeTarget", { targetId: result.targetId }) };
}

const FILE = `vault-${Date.now()}.redb`; // fresh file, so a rerun proves itself

console.log("--- session 1: create the vault on OPFS and write notes");
const first = await tab();
const created = JSON.parse(await first.evaluate(`api.open(${JSON.stringify(FILE)})`));
console.log(JSON.stringify(created));
await first.evaluate(`api.write("kept.md", "survives a reload")`);
await first.evaluate(`api.write("work/also-kept.md", "in a folder")`);
console.log("wrote:", await first.evaluate("api.dump()"));
await first.evaluate("api.flush()");
await first.close();
console.log("closed the tab (releases the OPFS lock)");

await new Promise((r) => setTimeout(r, 1500));

console.log("\n--- session 2: reopen the same file in a new tab, no peer");
const second = await tab();
const reboot = JSON.parse(await second.evaluate(`api.open(${JSON.stringify(FILE)})`));
console.log(JSON.stringify(reboot));
const reopened = JSON.parse(await second.evaluate("api.dump()"));
console.log("found:", JSON.stringify(reopened));

const has = (key, value) => reopened.some((e) => e.key === key && e.value === value);
const persisted =
  reopened.length === 2 &&
  has("kept.md", "survives a reload") &&
  has("work/also-kept.md", "in a folder");
console.log(persisted ? "PERSISTED across sessions" : "NOT PERSISTED");

// The durable content store is keyed by hash, so editing a note should leave
// the superseded version unreferenced, and the next boot should sweep it.
console.log("\n--- session 2 continued: edit a note, then check dedup + GC");
const before = await second.evaluate("api.blobsStored()");
await second.evaluate(`api.write("kept.md", "edited, so the old version is stale")`);
await second.evaluate(`api.write("kept.md", "edited, so the old version is stale")`);
const after = await second.evaluate("api.blobsStored()");
console.log(`blobs stored: ${before} -> ${after} (two writes, one new value)`);
const deduped = after === before + 1;
await second.evaluate("api.flush()");
await second.close();

await new Promise((r) => setTimeout(r, 1500));

console.log("\n--- session 3: reopen; the stale version should be collected");
const third = await tab();
const opened = JSON.parse(await third.evaluate(`api.open(${JSON.stringify(FILE)})`));
console.log(
  `restored ${opened.restored} blobs, GC dropped ${opened.gcDropped}, ${opened.blobsStored} remain`,
);
const final = JSON.parse(await third.evaluate("api.dump()"));
console.log("found:", JSON.stringify(final));
const collected = opened.gcDropped === 1 && opened.blobsStored === 2;

// The endpoint id is derived from the secret key, so an unchanged id across
// three sessions means peers see one device rather than three strangers.
console.log(
  `\nendpoint id per session: ${created.endpoint}, ${reboot.endpoint}, ${opened.endpoint}`,
);
const sameDevice =
  created.endpoint === reboot.endpoint && reboot.endpoint === opened.endpoint;

// iroh-docs mints a throwaway default author on every boot, since there is no
// way to hand it a remembered one. If that isn't cleaned up the author table
// grows by a key per launch, so this must stay at 1 rather than reaching 3.
console.log(`authors after each session: ${created.authors}, ${reboot.authors}, ${opened.authors}`);
const oneAuthor =
  created.authors === 1 && reboot.authors === 1 && opened.authors === 1;
const edited = final.some(
  (e) => e.key === "kept.md" && e.value === "edited, so the old version is stale",
);

// One writer per vault: a second tab on the same file must be refused with
// something a user can act on, rather than a raw OPFS error.
console.log("\n--- a second tab on the same vault");
const intruder = await tab();
let refusal = "(none)";
try {
  await intruder.evaluate(`api.open(${JSON.stringify(FILE)})`);
} catch (e) {
  refusal = String(e.message).slice(0, 200);
}
console.log("refused with:", refusal);
const refused = /already open in another tab/.test(refusal);

const ok = persisted && deduped && collected && edited && sameDevice && oneAuthor && refused;
console.log(
  `\npersisted=${persisted} deduped=${deduped} collected=${collected} edited=${edited} ` +
    `sameDevice=${sameDevice} oneAuthor=${oneAuthor} refused=${refused}`,
);
console.log(ok ? "ALL CHECKS PASSED" : "FAILED");
console.log("--- browser logs ---\n" + (logs.slice(0, 40).join("\n") || "(none)"));
process.exit(ok ? 0 : 1);
