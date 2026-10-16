//! Private vaults (#274): vaults that open only once the OS confirms the
//! device's owner is at it — Touch ID or the login password on macOS, the
//! fingerprint, face or PIN on Android.
//!
//! Which vaults are private is this device's choice, kept in `private.json` in
//! the app data dir; it doesn't sync. Which are unlocked lives only in memory,
//! so every launch starts locked. The lock is a gate, not encryption (#72): the
//! notes stay readable on disk. It lives here rather than in the UI alone so
//! the MCP server can refuse a locked vault too.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{AppHandle, State};

pub struct PrivateVaults {
    path: PathBuf,
    private: Mutex<HashSet<String>>,
    unlocked: Mutex<HashSet<String>>,
}

impl PrivateVaults {
    pub fn new(dir: PathBuf) -> Self {
        let path = dir.join("private.json");
        let private = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<HashSet<String>>(&s).ok())
            .unwrap_or_default();
        Self {
            path,
            private: Mutex::new(private),
            unlocked: Mutex::new(HashSet::new()),
        }
    }

    pub fn is_locked(&self, vault: &str) -> bool {
        self.private.lock().unwrap().contains(vault) && !self.unlocked.lock().unwrap().contains(vault)
    }

    pub fn lock_all(&self) {
        self.unlocked.lock().unwrap().clear();
    }

    fn save(&self, private: &HashSet<String>) -> Result<(), String> {
        let json = serde_json::to_string(private).map_err(|e| e.to_string())?;
        std::fs::write(&self.path, json).map_err(|e| format!("could not save private vaults: {e}"))
    }
}

const REASON: &str = "unlock a private vault";

/// The vaults this device keeps private.
#[tauri::command]
pub fn private_vaults(state: State<'_, PrivateVaults>) -> Vec<String> {
    state.private.lock().unwrap().iter().cloned().collect()
}

/// Make a vault private or not. Either way the owner confirms first: turning
/// it on proves this device can unlock at all before anything can get locked
/// out, and turning it off is as sensitive as opening it.
#[tauri::command]
pub async fn set_vault_private(
    app: AppHandle,
    state: State<'_, PrivateVaults>,
    vault: String,
    private: bool,
) -> Result<bool, String> {
    if !authenticate(&app, REASON).await? {
        return Ok(false);
    }
    let mut set = state.private.lock().unwrap().clone();
    if private {
        set.insert(vault.clone());
    } else {
        set.remove(&vault);
    }
    state.save(&set)?;
    *state.private.lock().unwrap() = set;
    // It's open in front of the person who just confirmed.
    state.unlocked.lock().unwrap().insert(vault);
    Ok(true)
}

/// Ask the OS to confirm the owner, then unlock `vault`. False when they
/// cancelled or failed.
#[tauri::command]
pub async fn unlock_vault(app: AppHandle, state: State<'_, PrivateVaults>, vault: String) -> Result<bool, String> {
    if !state.is_locked(&vault) {
        return Ok(true);
    }
    let ok = authenticate(&app, REASON).await?;
    if ok {
        state.unlocked.lock().unwrap().insert(vault);
    }
    Ok(ok)
}

/// Lock every vault but `except` — all of them when the app goes to the
/// background or sits idle, the ones left behind when switching vaults.
#[tauri::command]
pub fn lock_vaults(state: State<'_, PrivateVaults>, except: Option<String>) {
    state.unlocked.lock().unwrap().retain(|v| Some(v) == except.as_ref());
}

#[cfg(target_os = "macos")]
async fn authenticate(_app: &AppHandle, reason: &str) -> Result<bool, String> {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_local_authentication::{LAContext, LAPolicy};

    let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
    // Nothing Objective-C may be held across the await (it isn't Send), so the
    // context rides inside the reply block, which keeps it alive until macOS
    // answers.
    {
        let ctx = unsafe { LAContext::new() };
        // DeviceOwnerAuthentication: Touch ID where there is one, falling back
        // to the login password, so a Mac without Touch ID still works.
        let policy = LAPolicy::DeviceOwnerAuthentication;
        unsafe { ctx.canEvaluatePolicy_error(policy) }.map_err(|e| e.localizedDescription().to_string())?;
        let pending = Mutex::new(Some((tx, ctx.clone())));
        let reply = RcBlock::new(move |ok: Bool, _err: *mut NSError| {
            if let Some((tx, _ctx)) = pending.lock().unwrap().take() {
                let _ = tx.send(ok.as_bool());
            }
        });
        unsafe { ctx.evaluatePolicy_localizedReason_reply(policy, &NSString::from_str(reason), &reply) };
    }
    rx.await.map_err(|_| "unlock was interrupted".to_string())
}

#[cfg(any(target_os = "android", target_os = "ios"))]
async fn authenticate(app: &AppHandle, reason: &str) -> Result<bool, String> {
    use tauri_plugin_biometric::{AuthOptions, BiometricExt};

    let app = app.clone();
    let reason = reason.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        // allow_device_credential: the PIN or pattern works too, so a phone
        // without a fingerprint enrolled can still unlock.
        let options = AuthOptions {
            allow_device_credential: true,
            ..Default::default()
        };
        // The plugin reports a cancel the same way as a failure; both mean
        // "stay locked", and the person can try again.
        app.biometric().authenticate(reason, options).is_ok()
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "android", target_os = "ios")))]
async fn authenticate(_app: &AppHandle, _reason: &str) -> Result<bool, String> {
    Err("Private vaults need Touch ID or a device unlock, which this platform doesn't offer yet".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_vaults_start_locked_and_persist() {
        let dir = std::env::temp_dir().join(format!("vellum-private-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        let pv = PrivateVaults::new(dir.clone());
        assert!(!pv.is_locked("a"), "a vault nobody made private is never locked");

        let set: HashSet<String> = ["a".to_string()].into();
        pv.save(&set).unwrap();
        *pv.private.lock().unwrap() = set;
        assert!(pv.is_locked("a"));
        pv.unlocked.lock().unwrap().insert("a".into());
        assert!(!pv.is_locked("a"));

        // A relaunch reads the list back, and starts with everything locked.
        let again = PrivateVaults::new(dir.clone());
        assert!(again.is_locked("a"));
        assert!(!again.is_locked("b"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
