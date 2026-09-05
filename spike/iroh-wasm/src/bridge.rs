//! The worker's command surface, in Rust.
//!
//! Everything the browser vault decides is decided here, not in `worker.js`:
//! which commands exist, what one writer per vault means, and what the user is
//! told when that rule bites. `worker.js` is reduced to the one thing that
//! cannot be Rust — a `Worker` entry point has to be a JavaScript module (you
//! cannot point `new Worker()` at a `.wasm`), and wasm cannot instantiate
//! itself, so something must `import init` and call it.
//!
//! The message shape (`{ cmd, args } -> { ok, value } | { ok, error }`) is
//! deliberately the shape of the `VaultBackend` seam, so the real port swaps
//! the transport without moving any of this.

use js_sys::{Array, Function, Object, Promise, Reflect};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent};

fn scope() -> DedicatedWorkerGlobalScope {
    js_sys::global().unchecked_into()
}

fn key(name: &str) -> JsValue {
    JsValue::from_str(name)
}

/// Install the command handler, then answer anything that arrived while the
/// module was still compiling.
///
/// `backlog` is the reason the bootstrap is seven lines rather than four.
/// Instantiating 9.5 MB of wasm takes long enough that a page will send its
/// first command before `serve` can run, and a message posted to a worker whose
/// `onmessage` is still unset is **dropped, not queued** — the port's message
/// queue is enabled when the module starts evaluating, not when its top-level
/// `await` finishes. The first run of this bridge lost `open` exactly there and
/// hung. So the bootstrap parks arrivals in an array from its first synchronous
/// line, and hands them over here.
#[wasm_bindgen]
pub fn serve(backlog: Array) {
    console_error_panic_hook::set_once();
    let handler = Closure::<dyn FnMut(MessageEvent)>::new(|event: MessageEvent| {
        answer(event.data());
    });
    scope().set_onmessage(Some(handler.as_ref().unchecked_ref()));
    // The handler lives as long as the worker does.
    handler.forget();
    // Before anything newly arriving, since these were sent first.
    for request in backlog.iter() {
        answer(request);
    }
}

/// Run one command and post its reply, tagged with the request's id.
fn answer(request: JsValue) {
    spawn_local(async move {
        let reply = Object::new();
        let id = Reflect::get(&request, &key("id")).unwrap_or(JsValue::UNDEFINED);
        let _ = Reflect::set(&reply, &key("id"), &id);
        match dispatch(&request).await {
            Ok(value) => {
                let _ = Reflect::set(&reply, &key("ok"), &JsValue::TRUE);
                let _ = Reflect::set(&reply, &key("value"), &value);
            }
            Err(e) => {
                let _ = Reflect::set(&reply, &key("ok"), &JsValue::FALSE);
                let _ = Reflect::set(&reply, &key("error"), &message(&e));
            }
        }
        let _ = scope().post_message(&reply);
    });
}

async fn dispatch(request: &JsValue) -> Result<JsValue, JsValue> {
    let cmd = Reflect::get(request, &key("cmd"))?
        .as_string()
        .ok_or_else(|| JsValue::from_str("message had no cmd"))?;
    let args: Array = Reflect::get(request, &key("args"))?
        .dyn_into()
        .unwrap_or_else(|_| Array::new());
    let arg = |i: u32| args.get(i).as_string();
    let required = |i: u32| {
        arg(i).ok_or_else(|| JsValue::from_str(&format!("{cmd} is missing argument {i}")))
    };

    match cmd.as_str() {
        "open" => {
            let file = required(0)?;
            // Before the OPFS handle, so the refusal is ours rather than redb's.
            claim(&file).await?;
            crate::start_persistent(file, arg(1)).await.map(JsValue::from)
        }
        "write" => crate::write(required(0)?, required(1)?)
            .await
            .map(|()| JsValue::UNDEFINED),
        "dump" => crate::dump().await.map(JsValue::from),
        "blobsStored" => crate::blobs_stored().map(|n| JsValue::from(n as f64)),
        "flush" => crate::flush().await.map(|()| JsValue::UNDEFINED),
        other => Err(JsValue::from_str(&format!("unknown command {other}"))),
    }
}

/// What the second tab is told. A backend rule deserves a backend-owned
/// sentence — this used to be a string literal in `worker.js`.
const BUSY: &str = "This vault is already open in another tab. Close it there, or use that tab.";

/// One writer per vault.
///
/// OPFS sync access handles are an exclusive lock, so a second tab *will* be
/// refused; the only question is whether it is refused with something a user can
/// act on or with a raw `NoModificationAllowedError` from deep inside redb. A
/// Web Lock, taken first, answers that.
///
/// Reached through `Reflect` rather than `web_sys::LockManager` because those
/// bindings are gated behind `--cfg=web_sys_unstable_apis`, which every build of
/// this spike would then have to set. Same reason, and same shape, as
/// `opfs::open_handle` reaching `navigator.storage`.
async fn claim(vault: &str) -> Result<(), JsValue> {
    let locks = Reflect::get(&scope().navigator(), &key("locks"))?;
    let request: Function = Reflect::get(&locks, &key("request"))?.dyn_into()?;

    // A promise we settle ourselves, because `request`'s own promise is no use
    // here: on success our callback returns a promise that never settles — that
    // is precisely what holds the lock — so awaiting it would hang forever.
    let (mut hold, mut reject) = (None, None);
    let settled = Promise::new(&mut |resolve, rejects| {
        hold = Some(resolve);
        reject = Some(rejects);
    });
    let (hold, reject) = (
        hold.expect("Promise runs its executor synchronously"),
        reject.expect("Promise runs its executor synchronously"),
    );

    let granted = Closure::<dyn FnMut(JsValue) -> Promise>::new(move |lock: JsValue| {
        if lock.is_null() || lock.is_undefined() {
            let _ = reject.call1(&JsValue::UNDEFINED, &JsValue::from_str(BUSY));
            return Promise::resolve(&JsValue::UNDEFINED);
        }
        let _ = hold.call0(&JsValue::UNDEFINED);
        // Never settles, so the lock is held for as long as this worker lives,
        // and released when the tab goes away.
        Promise::new(&mut |_, _| {})
    });

    let options = Object::new();
    Reflect::set(&options, &key("mode"), &key("exclusive"))?;
    Reflect::set(&options, &key("ifAvailable"), &JsValue::TRUE)?;
    request.call3(
        &locks,
        &JsValue::from_str(&format!("vellum-vault:{vault}")),
        &options,
        granted.as_ref(),
    )?;
    // Outlives this call by design: the lock is held for the worker's lifetime.
    granted.forget();

    JsFuture::from(settled).await.map(|_| ())
}

/// Errors reach JS as a plain string, whatever shape they arrived in.
fn message(e: &JsValue) -> JsValue {
    if let Some(text) = e.as_string() {
        return JsValue::from_str(&text);
    }
    if let Some(text) = Reflect::get(e, &key("message")).ok().and_then(|m| m.as_string()) {
        return JsValue::from_str(&text);
    }
    JsValue::from_str(&format!("{e:?}"))
}
