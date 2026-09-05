// Does a browser vault survive a reload? (PERSISTENCE.md step 2.)
//
// Tab 1 opens a vault whose replica lives on OPFS, writes two notes, and is
// closed — which is what releases OPFS's exclusive lock, and models a reload.
// Tab 2 then opens the same file and must find those notes *with their content*
// and no peer involved: read off disk, not synced from anywhere.
//
// The replica comes from redb on an OPFS sync access handle; the content from
// the append-only blob log beside it (see src/opfs.rs).
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
console.log(await first.evaluate(`api.open(${JSON.stringify(FILE)})`));
await first.evaluate(`api.write("kept.md", "survives a reload")`);
await first.evaluate(`api.write("work/also-kept.md", "in a folder")`);
console.log("wrote:", await first.evaluate("api.dump()"));
await first.close();
console.log("closed the tab (releases the OPFS lock)");

await new Promise((r) => setTimeout(r, 1500));

console.log("\n--- session 2: reopen the same file in a new tab, no peer");
const second = await tab();
console.log(await second.evaluate(`api.open(${JSON.stringify(FILE)})`));
const reopened = JSON.parse(await second.evaluate("api.dump()"));
console.log("found:", JSON.stringify(reopened));

const has = (key, value) => reopened.some((e) => e.key === key && e.value === value);
const ok =
  reopened.length === 2 &&
  has("kept.md", "survives a reload") &&
  has("work/also-kept.md", "in a folder");
console.log(ok ? "\nPERSISTED across sessions" : "\nNOT PERSISTED");
console.log("--- browser logs ---\n" + (logs.slice(0, 40).join("\n") || "(none)"));
process.exit(ok ? 0 : 1);
