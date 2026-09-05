// Drives two independent browser tabs, each running its own iroh node in WASM:
// tab A creates a vault, tab B joins it from A's ticket, both write a note, and
// we then require each tab to see the other's note *with its content*.
//
// Run: see README.md. Exits 0 only if both directions synced.
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

async function tab(url) {
  const { result } = await send("Target.createTarget", { url });
  const attached = await send("Target.attachToTarget", { targetId: result.targetId, flatten: true });
  const sessionId = attached.result.sessionId;
  await send("Runtime.enable", {}, sessionId);
  const evaluate = async (expression) => {
    const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true }, sessionId);
    if (r.result?.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails).slice(0, 300));
    if (r.error) throw new Error(JSON.stringify(r.error).slice(0, 300));
    return r.result?.result?.value;
  };
  // Poll from here rather than inside the page: createTarget resolves before
  // navigation commits, and the pre-navigation context gets torn down under us.
  for (let i = 0; i < 300; i++) {
    try {
      if ((await evaluate("typeof window.api")) === "object") {
        await evaluate("window.ready");
        return { evaluate };
      }
    } catch {
      /* context replaced mid-navigation; try again */
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`wasm module never came up in ${url}`);
}

const a = await tab("http://127.0.0.1:9334/index.html");
const b = await tab("http://127.0.0.1:9334/index.html");
console.log("both tabs loaded the wasm module");

const created = JSON.parse(await a.evaluate("api.start()"));
console.log("tab A created a vault:", created.endpoint, "ticket", created.ticket.slice(0, 32) + "…");
const joined = JSON.parse(await b.evaluate(`api.start(${JSON.stringify(created.ticket)})`));
console.log("tab B joined as:", joined.endpoint);

await a.evaluate(`api.write("from-a.md", "written in tab A")`);
await b.evaluate(`api.write("from-b.md", "written in tab B")`);
console.log("both tabs wrote a note; waiting for sync…");

const sees = (dumped, key, value) => dumped.some((e) => e.key === key && e.value === value);
let ok = false;
for (let i = 0; i < 40; i++) {
  const da = JSON.parse(await a.evaluate("api.dump()"));
  const db = JSON.parse(await b.evaluate("api.dump()"));
  if (i % 5 === 0) console.log(`  t+${i / 2}s  A sees ${JSON.stringify(da)}  B sees ${JSON.stringify(db)}`);
  if (sees(da, "from-b.md", "written in tab B") && sees(db, "from-a.md", "written in tab A")) {
    console.log(`\nSYNCED both ways after ~${i / 2}s`);
    console.log("  A sees:", JSON.stringify(da));
    console.log("  B sees:", JSON.stringify(db));
    ok = true;
    break;
  }
  await new Promise((r) => setTimeout(r, 500));
}
if (!ok) console.log("\nNOT SYNCED within 20s");
console.log("--- browser logs ---\n" + (logs.slice(0, 40).join("\n") || "(none)"));
process.exit(ok ? 0 : 1);
