//! The command surface, in Rust.
//!
//! Mirrors the desktop's Tauri command names one-for-one, so `vault-wasm.ts`
//! reads like `vault-tauri.ts` with `call()` where `invoke()` would be. The
//! commands themselves are the portable functions in `vellum_vault::vault` —
//! the same code the desktop runs, not a second implementation.
//!
//! `worker.js` is the irreducible bootstrap: a `Worker` entry point has to be a
//! JavaScript module, wasm cannot instantiate itself, and a message posted
//! before `onmessage` is set is dropped rather than queued.

use js_sys::{Array, Object, Promise, Reflect, Uint8Array};
use serde::Serialize;
use vellum_vault::vault as v;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent};

use crate::{node, set_node};

fn scope() -> DedicatedWorkerGlobalScope {
    js_sys::global().unchecked_into()
}

fn key(name: &str) -> JsValue {
    JsValue::from_str(name)
}

/// Structured results cross as JSON rather than through serde-wasm-bindgen, to
/// keep the dependency list short; these payloads are small and the callers
/// already await a message round-trip.
fn json<T: Serialize>(value: &T) -> Result<JsValue, JsValue> {
    let text = serde_json::to_string(value).map_err(|e| JsValue::from_str(&e.to_string()))?;
    js_sys::JSON::parse(&text)
}

fn oops(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}

/// Install the command handler, then answer whatever arrived while the module
/// was still compiling. See `worker.js` for why the backlog exists.
#[wasm_bindgen]
pub fn serve(backlog: Array) {
    console_error_panic_hook::set_once();
    let handler = Closure::<dyn FnMut(MessageEvent)>::new(|event: MessageEvent| {
        answer(event.data());
    });
    scope().set_onmessage(Some(handler.as_ref().unchecked_ref()));
    handler.forget();
    for request in backlog.iter() {
        answer(request);
    }
}

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

/// Tell the page a vault changed. The desktop shell bridges the node's change
/// channel to a Tauri event; this bridges it to a worker message, and
/// `vault-wasm.ts` turns it back into the same `onVaultChanged` callback.
pub fn publish_change(vault: &str) {
    let msg = Object::new();
    let _ = Reflect::set(&msg, &key("event"), &key("vault-changed"));
    let _ = Reflect::set(&msg, &key("vault"), &key(vault));
    let _ = scope().post_message(&msg);
}

async fn dispatch(request: &JsValue) -> Result<JsValue, JsValue> {
    let cmd = Reflect::get(request, &key("cmd"))?
        .as_string()
        .ok_or_else(|| JsValue::from_str("message had no cmd"))?;
    let args: Array = Reflect::get(request, &key("args"))?
        .dyn_into()
        .unwrap_or_else(|_| Array::new());
    let s = |i: u32| -> Result<String, JsValue> {
        args.get(i)
            .as_string()
            .ok_or_else(|| JsValue::from_str(&format!("{cmd} is missing string argument {i}")))
    };
    let flag = |i: u32| args.get(i).as_bool().unwrap_or(false);

    // `open` is the browser's equivalent of the desktop building its node
    // lazily on first command: there is no app data dir to default to, so the
    // page names the vault database.
    if cmd == "open" {
        let file = s(0)?;
        claim(&file).await?;
        let n = crate::boot(&file).await.map_err(oops)?;
        set_node(n);
        crate::start_syncing().await.map_err(oops)?;
        return Ok(JsValue::UNDEFINED);
    }

    let n = node().ok_or_else(|| JsValue::from_str("open a vault database first"))?;
    match cmd.as_str() {
        "list_vaults" => json(&v::all_vaults(&n).await.map_err(oops)?),
        "create_vault" => {
            // The vault's name is a blob like any note's content, so this needs
            // persisting as much as a write does. Without it a vault created
            // and reloaded before its first note reads as "Waiting for a peer…"
            // — its name gone for good.
            let info = v::create(&n, s(0)?).await.map_err(oops)?;
            crate::persist_content(&n, &info.id).await.map_err(oops)?;
            json(&info)
        }
        "join_vault" => json(&v::join(&n, &s(0)?).await.map_err(oops)?),
        "share_vault" => Ok(JsValue::from_str(&v::share(&n, &s(0)?).await.map_err(oops)?)),
        "forget_vault" => v::forget(&n, &s(0)?).await.map(|_| JsValue::UNDEFINED).map_err(oops),
        "rename_vault" => v::rename(&n, &s(0)?, &s(1)?).map(|_| JsValue::UNDEFINED).map_err(oops),

        "list_tree" => json(&v::tree(&n, &s(0)?).await.map_err(oops)?),
        "read_note" => Ok(JsValue::from_str(&v::read(&n, &s(0)?, &s(1)?).await.map_err(oops)?)),
        "write_note" => {
            let (vault, path) = (s(0)?, s(1)?);
            v::write(&n, &vault, &path, &s(2)?, &s(3)?).await.map_err(oops)?;
            crate::persist_content(&n, &vault).await.map_err(oops)?;
            Ok(JsValue::UNDEFINED)
        }
        "create_note" => {
            let (vault, path) = (s(0)?, s(1)?);
            let free = v::create_note_at(&n, &vault, &path).await.map_err(oops)?;
            crate::persist_content(&n, &vault).await.map_err(oops)?;
            Ok(JsValue::from_str(&free))
        }
        "create_folder" => {
            let vault = s(0)?;
            v::create_folder_at(&n, &vault, &s(1)?).await.map_err(oops)?;
            crate::persist_content(&n, &vault).await.map_err(oops)?;
            Ok(JsValue::UNDEFINED)
        }
        "rename_path" => {
            let vault = s(0)?;
            v::rename_at(&n, &vault, &s(1)?, &s(2)?, flag(3)).await.map_err(oops)?;
            crate::persist_content(&n, &vault).await.map_err(oops)?;
            Ok(JsValue::UNDEFINED)
        }
        "delete_path" => {
            let vault = s(0)?;
            v::delete_at(&n, &vault, &s(1)?, flag(2)).await.map_err(oops)?;
            crate::persist_content(&n, &vault).await.map_err(oops)?;
            Ok(JsValue::UNDEFINED)
        }

        "export_vault" => {
            let bytes = v::export_zip(&n, &s(0)?).await.map_err(oops)?;
            Ok(Uint8Array::from(bytes.as_slice()).into())
        }
        "import_vault" => {
            let vault = s(0)?;
            let data: Uint8Array = args.get(1).dyn_into()?;
            let count = v::import_zip(&n, &vault, data.to_vec()).await.map_err(oops)?;
            crate::persist_content(&n, &vault).await.map_err(oops)?;
            Ok(JsValue::from_f64(count as f64))
        }

        "watch_vault" => {
            let nsid = v::parse_id(&s(0)?).map_err(oops)?;
            v::arm(&n, nsid).await.map(|_| JsValue::UNDEFINED).map_err(oops)
        }
        // The browser has no keep-alive to flip — a closed tab stops syncing
        // regardless — but arming every vault is the same work the desktop does,
        // and makes an open tab a hub for the vaults it holds.
        "set_background_sync" => {
            if flag(0) {
                v::arm_all(&n).await.map_err(oops)?;
            }
            Ok(JsValue::UNDEFINED)
        }

        "search_notes" => {
            let max = args.get(2).as_f64().unwrap_or(50.0) as usize;
            json(&v::search(&n, &s(0)?, &s(1)?, None, max).await.map_err(oops)?)
        }
        "list_tags" => json(&v::tags_in_vault(&n, &s(0)?).await.map_err(oops)?),
        "list_note_types" => json(&v::note_types(&n, &s(0)?).await.map_err(oops)?),

        other => Err(JsValue::from_str(&format!("unknown command {other}"))),
    }
}

/// What the second tab is told. A backend rule deserves a backend-owned
/// sentence.
const BUSY: &str = "This vault is already open in another tab. Close it there, or use that tab.";

/// One writer per vault.
///
/// OPFS sync access handles are an exclusive lock, so a second tab *will* be
/// refused; the only question is whether it is refused with something a user can
/// act on or with a raw `NoModificationAllowedError` from inside redb. A Web
/// Lock, taken first, answers that.
///
/// Reached through `Reflect` rather than `web_sys::LockManager` because those
/// bindings are gated behind `--cfg=web_sys_unstable_apis`, which every build
/// would then have to set.
async fn claim(vault: &str) -> Result<(), JsValue> {
    let locks = Reflect::get(&scope().navigator(), &key("locks"))?;
    let request: js_sys::Function = Reflect::get(&locks, &key("request"))?.dyn_into()?;

    // A promise we settle ourselves: `request`'s own promise is no use, because
    // on success our callback returns a promise that never settles — which is
    // precisely what holds the lock — so awaiting it would hang forever.
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
        // Never settles, so the lock is held for as long as this worker lives.
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
    granted.forget();

    JsFuture::from(settled).await.map(|_| ())
}

fn message(e: &JsValue) -> JsValue {
    if let Some(text) = e.as_string() {
        return JsValue::from_str(&text);
    }
    if let Some(text) = Reflect::get(e, &key("message")).ok().and_then(|m| m.as_string()) {
        return JsValue::from_str(&text);
    }
    JsValue::from_str(&format!("{e:?}"))
}
