mod vault;

use tauri::Manager;

/// Called from MainActivity.onCreate (Kotlin `external fun`). Tauri's Android
/// runtime never populates the `ndk-context` crate global, so libraries that
/// read it (iroh's network monitor) panic with "android context was not
/// initialized". We hand it the JavaVM + Application context here, before the
/// iroh node is lazily built on the first command.
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_com_andymitch_notes_MainActivity_initAndroidContext(
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // iroh's networking stack uses rustls with no built-in provider; install one
    // before anything spins up a TLS client (relay/discovery). Required on
    // Android, harmless if a provider is already set elsewhere.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Route iroh logs to Android logcat for cross-network debugging.
    #[cfg(target_os = "android")]
    {
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;
        let _ = tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new("warn,iroh=info"))
            .with(paranoid_android::layer("noteslog"))
            .try_init();
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::new("warn,iroh=info,notes_lib=debug"))
            .try_init();
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // The iroh node is built lazily on first command (see vault.rs):
            // on Android it must start after tao initializes the JNI context,
            // which happens once the event loop runs — after setup.
            let dir = app.path().app_data_dir()?;
            app.manage(vault::VaultManager::new(dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            vault::list_vaults,
            vault::create_vault,
            vault::join_vault,
            vault::share_vault,
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
