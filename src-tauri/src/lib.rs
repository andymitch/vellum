mod vault;

use tauri::Manager;

/// Called from MainActivity.onCreate (Kotlin `external fun`). Tauri's Android
/// runtime never populates the `ndk-context` crate global, so libraries that
/// read it (iroh's network monitor) panic with "android context was not
/// initialized". We hand it the JavaVM + Application context here, before the
/// iroh node is lazily built on the first command.
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_com_andymitch_vellum_MainActivity_initAndroidContext(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    context: jni::objects::JObject,
) {
    use jni::objects::JObject;
    let Ok(vm) = env.get_java_vm() else { return };
    let Ok(global) = env.new_global_ref(&context) else { return };
    // SAFETY: vm pointer is valid for the process; the context global ref is
    // leaked below so it outlives all readers.
    unsafe {
        ndk_context::initialize_android_context(
            vm.get_java_vm_pointer() as *mut std::ffi::c_void,
            JObject::as_raw(global.as_obj()) as *mut std::ffi::c_void,
        );
    }
    std::mem::forget(global);
}

/// Called from MainActivity's ConnectivityManager callback when the active
/// network changes. Android doesn't surface this to native code, so we notify
/// iroh explicitly to re-probe and migrate connections (e.g. wifi <-> cellular).
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_com_andymitch_vellum_MainActivity_notifyNetworkChange(
    _env: jni::JNIEnv,
    _class: jni::objects::JClass,
) {
    vault::notify_network_change();
}

/// Sync the native system-bar icon contrast to the web theme. The bars are
/// transparent (drawn behind the themed web header), so Android can't infer the
/// right icon color from its own light/dark mode — the frontend pushes it here
/// on every theme change. No-op off Android.
#[tauri::command]
fn set_dark_mode(dark: bool) {
    #[cfg(target_os = "android")]
    {
        let ctx = ndk_context::android_context();
        let Ok(vm) = (unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }) else {
            return;
        };
        let Ok(mut env) = vm.attach_current_thread() else {
            return;
        };
        let _ = env.call_static_method(
            "com/andymitch/vellum/MainActivity",
            "setDarkMode",
            "(Z)V",
            &[jni::objects::JValue::Bool(dark as u8)],
        );
    }
    #[cfg(not(target_os = "android"))]
    let _ = dark;
}

/// Fetch the device's Material You (Monet) tonal palette as a JSON string of
/// `#RRGGBB` hex tones, for the frontend's "Dynamic" theme. Returns None off
/// Android and on Android < 12 (no dynamic colors).
#[tauri::command]
fn get_material_you() -> Option<String> {
    #[cfg(target_os = "android")]
    {
        let ctx = ndk_context::android_context();
        let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }.ok()?;
        let mut env = vm.attach_current_thread().ok()?;
        let result = env
            .call_static_method(
                "com/andymitch/vellum/MainActivity",
                "getDynamicColors",
                "()Ljava/lang/String;",
                &[],
            )
            .ok()?;
        let obj = result.l().ok()?;
        if obj.is_null() {
            return None;
        }
        let s: String = env
            .get_string(&jni::objects::JString::from(obj))
            .ok()?
            .into();
        Some(s)
    }
    #[cfg(not(target_os = "android"))]
    None
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // iroh's networking stack uses rustls with no built-in provider; install one
    // before anything spins up a TLS client (relay/discovery). Required on
    // Android, harmless if a provider is already set elsewhere.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Log filter. Desktop honors RUST_LOG (e.g. `iroh_gossip=debug` to debug
    // cross-network sync); Android logs to logcat tag `noteslog` at a quiet
    // default. Both fall back to warn + our own info.
    const DEFAULT_LOG: &str = "warn,notes_lib=info";
    #[cfg(target_os = "android")]
    let filter = tracing_subscriber::EnvFilter::new(DEFAULT_LOG);
    #[cfg(not(target_os = "android"))]
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(DEFAULT_LOG));

    #[cfg(target_os = "android")]
    {
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(paranoid_android::layer("noteslog"))
            .try_init();
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .try_init();
    }

    let builder = tauri::Builder::default().plugin(tauri_plugin_opener::init());
    // Native camera QR scanner for joining vaults (mobile only; the crate is
    // gated to android/ios in Cargo.toml).
    #[cfg(mobile)]
    let builder = builder.plugin(tauri_plugin_barcode_scanner::init());
    // In-app auto-update (desktop only; crates gated in Cargo.toml).
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init());
    builder
        .setup(|app| {
            // The iroh node is built lazily on first command (see vault.rs):
            // on Android it must start after tao initializes the JNI context,
            // which happens once the event loop runs — after setup.
            let dir = app.path().app_data_dir()?;
            app.manage(vault::VaultManager::new(dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            set_dark_mode,
            get_material_you,
            vault::list_vaults,
            vault::create_vault,
            vault::join_vault,
            vault::share_vault,
            vault::forget_vault,
            vault::list_tree,
            vault::read_note,
            vault::write_note,
            vault::create_note,
            vault::create_folder,
            vault::rename_path,
            vault::delete_path,
            vault::watch_vault,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
