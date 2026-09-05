// Browser <-> native sync: the pairing that matters for #221/#222 (a phone
// browser syncing with the installed desktop app).
//
// The native peer (spike/iroh-native-peer) must already be running: it writes a
// note, serves its share ticket on 127.0.0.1:8899, and waits for the browser's
// write. This script drives one browser tab: fetch the ticket, join, read the
// native note *with content*, write one back. The native peer prints HOST
// SUCCESS when the browser's note lands, so both directions are checked.
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

const { result } = await send("Target.createTarget", { url: "http://127.0.0.1:9334/index.html" });
const sessionId = (await send("Target.attachToTarget", { targetId: result.targetId, flatten: true }))
  .result.sessionId;
await send("Runtime.enable", {}, sessionId);
const evaluate = async (expression) => {
  const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true }, sessionId);
  if (r.result?.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails).slice(0, 300));
  return r.result?.result?.value;
};
// Poll from here: createTarget resolves before navigation commits.
for (let i = 0; ; i++) {
  try {
    if ((await evaluate("typeof window.api")) === "object") break;
  } catch {
    /* context replaced mid-navigation */
  }
  if (i > 300) throw new Error("wasm module never came up");
  await new Promise((r) => setTimeout(r, 100));
}
console.log("browser tab loaded the wasm module");

const ticket = await evaluate(`fetch("http://127.0.0.1:8899").then(r => r.text())`);
console.log("fetched the native peer's ticket:", ticket.slice(0, 32) + "…");
const joined = JSON.parse(await evaluate(`api.start(${JSON.stringify(ticket)})`));
console.log("browser joined the desktop vault as:", joined.endpoint);
await evaluate(`api.write("from-browser.md", "written in a browser")`);
console.log("browser wrote a note; waiting for the desktop's note to arrive…");

let ok = false;
for (let i = 0; i < 60; i++) {
  const dump = JSON.parse(await evaluate("api.dump()"));
  if (i % 5 === 0) console.log(`  t+${i / 2}s  browser sees ${JSON.stringify(dump)}`);
  if (dump.some((e) => e.key === "from-native.md" && e.value === "written on the desktop")) {
    console.log(`\nBROWSER READ THE DESKTOP'S NOTE after ~${i / 2}s`);
    console.log("  browser sees:", JSON.stringify(dump));
    ok = true;
    break;
  }
  await new Promise((r) => setTimeout(r, 500));
}
if (!ok) console.log("\nNOT SYNCED within 30s");
console.log("--- browser logs ---\n" + (logs.slice(0, 40).join("\n") || "(none)"));
process.exit(ok ? 0 : 1);
