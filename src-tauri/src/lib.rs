mod vault;

use tauri::Manager;

// The app class loader, cached on the Android UI thread during initAndroidContext.
// Worker threads (async Tauri commands run on tokio) JNI-attach with the *system*
// class loader, which can't resolve app classes by name — so we resolve them via
// this loader's loadClass() instead. (Sync commands run on the UI thread and can
// FindClass directly, which is why set_dark_mode works by name.)
#[cfg(target_os = "android")]
static APP_CLASS_LOADER: std::sync::OnceLock<jni::objects::GlobalRef> =
    std::sync::OnceLock::new();

// Guards one-time Android context init. With Background sync on, the process can
// outlive its Activity (we prevent the exit on swipe-away so the iroh node keeps
// running). A relaunched Activity calls initAndroidContext again, but
// ndk_context::initialize_android_context panics if called twice — and the cached
// VM/context/loader are still valid for the (same) process — so we init only once.
#[cfg(target_os = "android")]
static ANDROID_CTX_INIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// Set when we prevent the process exit on Activity destroy (Background sync on,
// swipe-away). The WebView is gone but the process + iroh node live on. Tauri
// can't rebuild the WebView in this process, so a relaunched Activity would show
// a blank screen — MainActivity checks this (survivedBackgroundExit) and restarts
// the app fresh for a working UI. See the run() exit handler.
#[cfg(target_os = "android")]
static EXIT_PREVENTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// Called from MainActivity.onCreate. True if this process kept itself alive
// through a prior Activity destroy for background sync (see EXIT_PREVENTED), i.e.
// the Activity is being recreated into a process whose WebView can't be rebuilt —
// the caller should relaunch fresh.
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_com_andymitch_vellum_MainActivity_survivedBackgroundExit(
    _env: jni::JNIEnv,
    _class: jni::objects::JClass,
) -> jni::sys::jboolean {
    EXIT_PREVENTED.load(std::sync::atomic::Ordering::Relaxed) as jni::sys::jboolean
}

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
    // Already initialized for this process (e.g. the Activity was recreated while
    // Background sync kept the process alive) — skip; re-initializing panics.
    if ANDROID_CTX_INIT.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let Ok(vm) = env.get_java_vm() else { return };
    let Ok(global) = env.new_global_ref(&context) else { return };
    // Cache the app class loader (we're on the UI thread here) so worker threads
    // can resolve app classes by name via loadClass — see APP_CLASS_LOADER.
    if let Ok(loader) = env.call_method(
        &context,
        "getClassLoader",
        "()Ljava/lang/ClassLoader;",
        &[],
    ) {
        if let Ok(obj) = loader.l() {
            if let Ok(g) = env.new_global_ref(&obj) {
                let _ = APP_CLASS_LOADER.set(g);
            }
        }
    }
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

/// Called from MainActivity.onResume. The OS freezes the process while
/// backgrounded, leaving iroh's sockets/relay connections stale with no native
/// signal. Re-arm sync so it recovers on foreground without a restart.
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_com_andymitch_vellum_MainActivity_notifyResume(
    _env: jni::JNIEnv,
    _class: jni::objects::JClass,
) {
    vault::on_resume();
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

// Whether Background sync is on. Desktop: closing the window hides to the tray
// (keep syncing) instead of quitting. Android: prevent the process from exiting
// when the Activity is destroyed (swipe-away), so the in-process iroh node keeps
// running under the foreground service. Mirrors the setting; see set_background_sync.
#[cfg(any(desktop, target_os = "android"))]
static LIVE_SYNC: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// Start/stop the Android foreground service (keeps the process + iroh node alive
// while backgrounded) via MainActivity.setBackgroundSync. Resolves the class
// through the cached app class loader so it works from any thread (async commands
// run on tokio workers whose system class loader can't find app classes by name).
#[cfg(target_os = "android")]
fn set_android_background_service(enabled: bool) {
    let ctx = ndk_context::android_context();
    let Ok(vm) = (unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }) else {
        return;
    };
    let Ok(mut env) = vm.attach_current_thread() else {
        return;
    };
    let Some(loader) = APP_CLASS_LOADER.get() else {
        tracing::warn!("no cached app classloader");
        return;
    };
    let res = (|| -> jni::errors::Result<()> {
        let name = env.new_string("com.andymitch.vellum.MainActivity")?;
        let class = env
            .call_method(
                loader.as_obj(),
                "loadClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                &[jni::objects::JValue::Object(name.as_ref())],
            )?
            .l()?;
        let class = jni::objects::JClass::from(class);
        env.call_static_method(
            &class,
            "setBackgroundSync",
            "(Z)V",
            &[jni::objects::JValue::Bool(enabled as u8)],
        )?;
        Ok(())
    })();
    // Clear any pending Java exception so it doesn't crash on return to the JVM.
    if res.is_err() {
        let _ = env.exception_clear();
    }
}

/// Toggle background "live sync" (the Background sync setting). Arms every vault
/// so this device is an always-on hub, then flips the platform keep-alive that
/// lets it keep syncing with no window open / while backgrounded:
///   - desktop: closing the window hides to the tray instead of quitting, and
///     Vellum is registered to launch at login (so a desktop hub survives reboots).
///   - Android: a foreground service holds the process alive in the background.
/// The frontend calls this on toggle and once on launch if it was left enabled.
#[tauri::command]
async fn set_background_sync(
    app: tauri::AppHandle,
    state: tauri::State<'_, vault::VaultManager>,
    enabled: bool,
) -> Result<(), String> {
    #[cfg(any(desktop, target_os = "android"))]
    LIVE_SYNC.store(enabled, std::sync::atomic::Ordering::Relaxed);
    #[cfg(desktop)]
    apply_desktop_background_sync(&app, enabled);
    #[cfg(target_os = "android")]
    set_android_background_service(enabled);
    if enabled {
        vault::arm_all_vaults(&app, state.inner()).await?;
    }
    Ok(())
}

// Apply the desktop side of the Background sync setting: remember it (for
// close-to-tray), show/hide the tray icon (visible only while on, so it's not
// menu-bar clutter), and register/unregister launch-at-login. Shared by the
// command and the tray's "Turn off" item.
#[cfg(desktop)]
fn apply_desktop_background_sync(app: &tauri::AppHandle, enabled: bool) {
    use std::sync::atomic::Ordering;
    use tauri_plugin_autostart::ManagerExt;
    LIVE_SYNC.store(enabled, Ordering::Relaxed);
    if let Some(tray) = app.try_state::<tauri::tray::TrayIcon>() {
        let _ = tray.set_visible(enabled);
    }
    let autostart = app.autolaunch();
    let _ = if enabled {
        autostart.enable()
    } else {
        autostart.disable()
    };
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // iroh's networking stack uses rustls with no built-in provider; install one
    // before anything spins up a TLS client (relay/discovery). Required on
    // Android, harmless if a provider is already set elsewhere.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Log filter. Desktop honors RUST_LOG (e.g. `iroh_gossip=debug` to debug
    // cross-network sync); Android logs to logcat tag `vellum` at a quiet
    // default. Both fall back to warn + our own info.
    const DEFAULT_LOG: &str = "warn,vellum_lib=info";
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
            .with(paranoid_android::layer("vellum"))
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
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        // Launch at login (toggled by Background sync) so an always-on desktop
        // stays a sync hub across reboots. The "--autostart" arg lets us start
        // hidden to the tray on a login launch (handled in setup).
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ));
    // Closing the window quits by default. With Background sync on, hide to the
    // tray instead so the iroh node keeps syncing in the background.
    #[cfg(desktop)]
    let builder = builder.on_window_event(|window, event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            if LIVE_SYNC.load(std::sync::atomic::Ordering::Relaxed) {
                api.prevent_close();
                let _ = window.hide();
            }
        }
    });
    // macOS Cmd+Q is the other "implicit quit" path. The native Quit menu item
    // calls -[NSApplication terminate:], which tao delivers straight as
    // RunEvent::Exit with NO RunEvent::ExitRequested first — so prevent_exit()
    // can't catch it. We replace the app menu's Quit with our own item (id
    // "menu_quit") so Cmd+Q routes through on_menu_event, where (like the close
    // button) we hide to the tray while Background sync is on. The tray's "Quit
    // Vellum" still hard-quits via app.exit(0). See setup_macos_menu.
    #[cfg(target_os = "macos")]
    let builder = builder.on_menu_event(|app, ev| {
        if ev.id().as_ref() == "menu_quit" {
            if LIVE_SYNC.load(std::sync::atomic::Ordering::Relaxed) {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            } else {
                app.exit(0);
            }
        }
    });
    builder
        .setup(|app| {
            // The iroh node is built lazily on first command (see vault.rs):
            // on Android it must start after tao initializes the JNI context,
            // which happens once the event loop runs — after setup.
            let dir = app.path().app_data_dir()?;
            app.manage(vault::VaultManager::new(dir));
            #[cfg(desktop)]
            setup_tray(app)?;
            #[cfg(target_os = "macos")]
            setup_macos_menu(app)?;
            // Started at login → stay hidden in the tray (don't pop a window).
            #[cfg(desktop)]
            if std::env::args().any(|a| a == "--autostart") {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }
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
            set_background_sync,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|_app, _event| {
            // Android: when the Activity is destroyed (e.g. swiped from recents),
            // tao's event loop returns and calls std::process::exit, taking the
            // in-process iroh node with it. With Background sync on we prevent that
            // exit so the node keeps syncing under the foreground service.
            // (Desktop's quit paths are handled by the window/menu handlers above.)
            #[cfg(target_os = "android")]
            if let tauri::RunEvent::ExitRequested { api, .. } = &_event {
                if LIVE_SYNC.load(std::sync::atomic::Ordering::Relaxed) {
                    tracing::info!("android: preventing exit to keep background sync alive");
                    api.prevent_exit();
                    EXIT_PREVENTED.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
        });
}

// Bring the main window to the front (tray left-click / "Open Vellum").
#[cfg(desktop)]
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

// System-tray icon (desktop). Shown only while Background sync is on (it's the
// only time the app runs without a window), so it doubles as the daemon's
// presence + control surface: a status line, Open, Turn off background sync, and
// Quit. Built hidden at startup and managed as app state; visibility is toggled
// by apply_desktop_background_sync.
#[cfg(desktop)]
fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::image::Image;
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use tauri::Emitter;

    // Disabled header so the menu reads as the daemon's status.
    let status = MenuItem::with_id(app, "status", "Background sync: on", false, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let show = MenuItem::with_id(app, "show", "Open Vellum", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "Turn off background sync", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Vellum", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&status, &sep, &show, &stop, &quit])?;

    // The logo as a monochrome template image so macOS tints it to the menu-bar
    // appearance (light/dark) instead of showing the full-color app icon.
    let icon = Image::from_bytes(include_bytes!("../icons/tray-icon.png"))?;

    let tray = TrayIconBuilder::new()
        .icon(icon)
        .icon_as_template(true)
        .tooltip("Vellum — background sync")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, ev| match ev.id.as_ref() {
            "show" => show_main_window(app),
            "stop" => {
                apply_desktop_background_sync(app, false);
                // Keep the in-app Settings toggle in sync with the tray action.
                let _ = app.emit("background-sync", false);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    // Hidden until Background sync turns it on (see apply_desktop_background_sync).
    let _ = tray.set_visible(false);
    app.manage(tray);
    Ok(())
}

// Build the macOS app menu. This mirrors Tauri's default menu but swaps the
// native Quit (a PredefinedMenuItem that calls -[NSApplication terminate:],
// which can't be intercepted — see the on_menu_event note in run()) for a custom
// "menu_quit" item carrying the Cmd+Q accelerator, so Background sync can hide to
// the tray on Cmd+Q instead of dying. The Edit/Window submenus are re-added so
// text-editing shortcuts (copy/paste/undo/select-all) and window controls keep
// working once we take over the menu.
#[cfg(target_os = "macos")]
fn setup_macos_menu(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
    let quit = MenuItem::with_id(app, "menu_quit", "Quit Vellum", true, Some("Cmd+Q"))?;
    let app_menu = Submenu::with_items(
        app,
        "Vellum",
        true,
        &[
            &PredefinedMenuItem::about(app, None, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::services(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let edit_menu = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let window_menu = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
            &PredefinedMenuItem::fullscreen(app, None)?,
        ],
    )?;
    let menu = Menu::with_items(app, &[&app_menu, &edit_menu, &window_menu])?;
    app.set_menu(menu)?;
    Ok(())
}
