#[cfg(desktop)]
mod mcp;
mod link_preview;
mod share;
mod vault;

use tauri::Manager;

// Guards one-time Android context init. An Activity can be recreated within the
// same process (e.g. a config change), which would call initAndroidContext again,
// but ndk_context::initialize_android_context panics if called twice — and the
// cached VM/context are still valid for the (same) process — so we init only once.
#[cfg(target_os = "android")]
static ANDROID_CTX_INIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Called from MainActivity.onCreate (Kotlin `external fun`). Tauri's Android
/// runtime never populates the `ndk-context` crate global, so libraries that
/// read it (iroh's network monitor) panic with "android context was not
/// initialized". We hand it the JavaVM + Application context here, before the
/// iroh node is lazily built on the first command.
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_com_andymitch_vellum_MainActivity_initAndroidContext(
    env: jni::JNIEnv,
    _class: jni::objects::JClass,
    context: jni::objects::JObject,
) {
    use jni::objects::JObject;
    // Already initialized for this process (e.g. the Activity was recreated by a
    // config change) — skip; re-initializing panics.
    if ANDROID_CTX_INIT.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
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

/// Toggle Android immersive mode (hide/show the status bar) to follow the web
/// chrome auto-hide on scroll (#85). When hidden, bars reappear transiently on a
/// swipe from the edge. No-op off Android. Sync command → runs on the Android
/// main thread, so the app class can be resolved by name (see set_dark_mode).
#[tauri::command]
fn set_immersive(hidden: bool) {
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
            "setStatusBarHidden",
            "(Z)V",
            &[jni::objects::JValue::Bool(hidden as u8)],
        );
    }
    #[cfg(not(target_os = "android"))]
    let _ = hidden;
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

/// Open the update download page (#145). On Android, routes to the Komi Store app
/// if installed (it intercepts github.com repo URLs), else the browser — see
/// MainActivity.openUpdatePage. Sync command → runs on the Android main thread, so
/// the app class resolves by name (see set_dark_mode). No-op off Android (desktop
/// uses the opener plugin directly).
#[tauri::command]
fn open_update_page(url: String) {
    #[cfg(target_os = "android")]
    {
        let ctx = ndk_context::android_context();
        let Ok(vm) = (unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }) else {
            return;
        };
        let Ok(mut env) = vm.attach_current_thread() else {
            return;
        };
        let Ok(jurl) = env.new_string(&url) else {
            return;
        };
        let _ = env.call_static_method(
            "com/andymitch/vellum/MainActivity",
            "openUpdatePage",
            "(Ljava/lang/String;)V",
            &[jni::objects::JValue::Object(&jurl)],
        );
    }
    #[cfg(not(target_os = "android"))]
    let _ = url;
}

// Whether Background sync is on (desktop only). When on, closing the window hides
// the app to the menu-bar tray instead of quitting, so the in-process iroh node
// keeps syncing as an always-on hub. Mirrors the setting; see set_background_sync.
#[cfg(desktop)]
static LIVE_SYNC: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Toggle background "live sync" (the desktop Background sync setting). Arms every
/// vault so this device is an always-on hub, then flips the desktop keep-alive
/// that lets it keep syncing with no window open: closing the window hides to the
/// menu-bar tray instead of quitting, and Vellum is registered to launch at login
/// (so a desktop hub survives reboots). The frontend calls this on toggle and once
/// on launch if it was left enabled. No-op on mobile (the toggle is desktop-only).
#[tauri::command]
async fn set_background_sync(
    app: tauri::AppHandle,
    state: tauri::State<'_, vault::VaultManager>,
    enabled: bool,
) -> Result<(), String> {
    #[cfg(desktop)]
    apply_desktop_background_sync(&app, enabled);
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
    // Turned the hub off while running window-less (menu-bar agent): nothing left
    // to keep the process around for, so exit cleanly instead of lingering invisibly.
    if !enabled && app.webview_windows().is_empty() {
        app.exit(0);
    }
}

// Was the hub left enabled? Launch-at-login is our persisted signal — it's set
// and cleared together with the Background sync setting (see above), so an
// always-on desktop re-arms itself on a login (or manual) launch.
#[cfg(desktop)]
fn hub_enabled_at_launch(app: &tauri::AppHandle) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().unwrap_or(false)
}

/// Current state of the local MCP server, for the Settings panel: whether it's
/// listening, on which port, and the ready-to-paste connect command.
#[cfg(desktop)]
#[tauri::command]
fn mcp_status(app: tauri::AppHandle) -> mcp::McpStatus {
    mcp::status(&app)
}

/// Toggle the local MCP server (the Agent access setting). Starting it binds a
/// loopback port; stopping it drops the listener and any live agent sessions.
/// The choice is persisted, so a relaunch restores it.
#[cfg(desktop)]
#[tauri::command]
async fn set_mcp_enabled(
    app: tauri::AppHandle,
    enabled: bool,
) -> Result<mcp::McpStatus, String> {
    mcp::set_enabled(&app, enabled).await
}

// Mobile has no MCP server (see mcp.rs for why), but the command surface stays
// uniform so `generate_handler!` doesn't have to be duplicated per platform.
// The Settings row that calls these is desktop-only anyway.
#[cfg(not(desktop))]
#[tauri::command]
fn mcp_status() -> serde_json::Value {
    serde_json::json!({ "enabled": false, "port": null, "url": null, "token": "", "command": null })
}

#[cfg(not(desktop))]
#[tauri::command]
fn set_mcp_enabled(_enabled: bool) -> Result<serde_json::Value, String> {
    Err("the MCP server is desktop-only".into())
}

/// Check for an update at a specific endpoint (#175).
///
/// The updater's endpoint is compiled into tauri.conf.json and points at
/// /releases/latest/, which EXCLUDES pre-releases — that is what keeps a beta
/// away from stable users, and the reason a beta tester can't be updated to the
/// next beta by the normal path. Only the Rust builder can override the
/// endpoint (the JS `check()` takes no such option), so the beta channel has to
/// come through here.
///
/// The caller resolves the URL, because the frontend already queries the GitHub
/// releases API for the changelog — doing it there keeps an HTTP client out of
/// the Rust side entirely.
///
/// Returns the available version, or None when already up to date.
#[cfg(desktop)]
#[tauri::command]
async fn check_update_at(app: tauri::AppHandle, url: String) -> Result<Option<String>, String> {
    use tauri_plugin_updater::UpdaterExt;
    let endpoint = url.parse().map_err(|e| format!("bad update url: {e}"))?;
    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;
    let update = updater.check().await.map_err(|e| e.to_string())?;
    Ok(update.map(|u| u.version))
}

/// Download and install the update at `url`, then the caller relaunches.
/// Re-checks rather than caching the Update between commands, so a stale handle
/// can never be installed after the user sat on the prompt.
#[cfg(desktop)]
#[tauri::command]
async fn install_update_at(app: tauri::AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let endpoint = url.parse().map_err(|e| format!("bad update url: {e}"))?;
    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        return Err("no update available".into());
    };
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())
}

// Mobile has no in-app updater at all, so these keep the command surface uniform
// rather than forking generate_handler! per platform (same reasoning as the MCP
// commands above).
#[cfg(not(desktop))]
#[tauri::command]
async fn check_update_at(_url: String) -> Result<Option<String>, String> {
    Ok(None)
}

#[cfg(not(desktop))]
#[tauri::command]
async fn install_update_at(_url: String) -> Result<(), String> {
    Err("updates are desktop-only".into())
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

    let builder = tauri::Builder::default();
    // Single-instance must be registered first (plugins run in registration order)
    // so a second launch is intercepted before any window/node setup. Desktop only:
    // it re-opens the running agent's window instead of spawning a rival process
    // that would contend for the iroh-docs store.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
        show_main_window(app);
    }));
    let builder = builder
        .plugin(tauri_plugin_opener::init())
        // Markdown export/import (#79) — dialog picks the file, fs reads/writes
        // it (incl. Android SAF). Both cross-platform.
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init());
    // Native camera QR scanner for joining vaults (mobile only; the crate is
    // gated to android/ios in Cargo.toml).
    #[cfg(mobile)]
    let builder = builder.plugin(tauri_plugin_barcode_scanner::init());
    // In-app auto-update (desktop only; crates gated in Cargo.toml).
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        // Launch at login (toggled by Background sync) so an always-on desktop
        // stays a sync hub across reboots. The "--autostart" arg lets us start
        // hidden to the tray on a login launch (handled in setup).
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ));
    // Closing the window destroys it (frees the webview). With the hub on, the
    // process survives the last window closing — see the ExitRequested handler in
    // run() — so the iroh node keeps syncing as a menu-bar agent. With it off, the
    // last window closing exits normally.
    //
    // macOS Cmd+Q is the other "implicit quit" path. The native Quit menu item
    // calls -[NSApplication terminate:], delivered straight as RunEvent::Exit with
    // no preventable ExitRequested — so we swap the app menu's Quit for our own
    // "menu_quit" item (see setup_macos_menu) and route it here: with the hub on,
    // just close the window (the agent lives on); otherwise quit. The tray's "Quit
    // Vellum" hard-quits via app.exit(0).
    #[cfg(target_os = "macos")]
    let builder = builder.on_menu_event(|app, ev| {
        if ev.id().as_ref() == "menu_quit" {
            if LIVE_SYNC.load(std::sync::atomic::Ordering::Relaxed) {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.close();
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
            app.manage(vault::VaultManager::new(dir.clone()));
            // Local MCP server (#164). Off unless the user turned it on; the
            // listener is loopback-only and token-authenticated either way.
            #[cfg(desktop)]
            {
                app.manage(mcp::McpServer::new(dir));
                mcp::start_if_enabled(app.handle());
            }
            #[cfg(desktop)]
            setup_tray(app)?;
            #[cfg(target_os = "macos")]
            setup_macos_menu(app)?;
            // Re-arm the always-on hub if it was left enabled (launch-at-login is
            // our persisted signal). Builds the iroh node + arms every vault with
            // no window/frontend needed, so a login launch syncs headlessly.
            #[cfg(desktop)]
            {
                let handle = app.handle().clone();
                if hub_enabled_at_launch(&handle) {
                    apply_desktop_background_sync(&handle, true);
                    let h = handle.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Some(state) = h.try_state::<vault::VaultManager>() {
                            let _ = vault::arm_all_vaults(&h, state.inner()).await;
                        }
                    });
                }
                // Started at login → run window-less as a menu-bar agent: drop the
                // auto-created window (frees the webview) and hide from the Dock.
                if std::env::args().any(|a| a == "--autostart") {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.close();
                    }
                    #[cfg(target_os = "macos")]
                    let _ = handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
                }
            }
            // Center the Overlay-titlebar traffic lights in our web header (#156).
            // The config-created launch window isn't built by create_main_window,
            // so arm it here (skipped on an --autostart launch, which has no window).
            #[cfg(target_os = "macos")]
            if let Some(w) = app.get_webview_window("main") {
                arm_traffic_light_centering(&w);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            set_dark_mode,
            set_immersive,
            get_material_you,
            open_update_page,
            vault::list_vaults,
            vault::create_vault,
            vault::join_vault,
            vault::share_vault,
            vault::forget_vault,
            vault::rename_vault,
            vault::list_tree,
            vault::read_note,
            vault::search_notes,
            vault::list_tags,
            vault::list_note_types,
            vault::write_note,
            vault::create_note,
            vault::export_vault,
            vault::import_vault,
            share::share_note,
            link_preview::fetch_link_preview,
            vault::create_folder,
            vault::rename_path,
            vault::delete_path,
            vault::watch_vault,
            set_background_sync,
            mcp_status,
            set_mcp_enabled,
            check_update_at,
            install_update_at,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|_app_handle, _event| {
            // Keep the always-on hub alive when the last window closes. ExitRequested
            // with code = None is an implicit exit (last window closed / Cmd+Q); with
            // the hub on we prevent it so the iroh node keeps syncing as a menu-bar
            // agent, and drop the Dock icon. Explicit app.exit(n) carries code = Some
            // (tray "Quit") and is never prevented.
            #[cfg(desktop)]
            if let tauri::RunEvent::ExitRequested { code, api, .. } = &_event {
                if code.is_none() && LIVE_SYNC.load(std::sync::atomic::Ordering::Relaxed) {
                    api.prevent_exit();
                    #[cfg(target_os = "macos")]
                    let _ = _app_handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
                }
            }
        });
}

// (Re)create the main window. Used to bring it back after a close destroyed it
// (the hub keeps running window-less). Mirrors the window config in
// tauri.conf.json — the config window is the one created at a normal launch; this
// is only for re-opening from the tray / a second launch.
#[cfg(desktop)]
fn create_main_window(app: &tauri::AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};
    #[allow(unused_mut)]
    let mut b = WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
        .title("Vellum")
        .inner_size(800.0, 600.0)
        .disable_drag_drop_handler();
    #[cfg(target_os = "macos")]
    {
        b = b
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true);
    }
    let w = b.build()?;
    #[cfg(target_os = "macos")]
    arm_traffic_light_centering(&w);
    Ok(w)
}

// The Overlay-titlebar traffic lights are standard macOS size and AppKit centers
// them for the 32px native titlebar. Our web header is taller (~37px), so they
// sit a few px above its vertical center. Nudge each button down to line up with
// the header content (#156). macOS resets the button positions on live-resize and
// fullscreen transitions, so callers re-apply via `arm_traffic_light_centering`.
#[cfg(target_os = "macos")]
fn center_traffic_lights(window: &tauri::WebviewWindow) {
    use objc2_app_kit::{NSWindow, NSWindowButton};
    use objc2_foundation::NSPoint;
    // Matches the macOS header height in App.svelte (min-h-0 + 28px mode-toggle
    // pill + 0.25rem vertical padding + 1px border).
    const HEADER_H: f64 = 37.0;
    let Ok(ptr) = window.ns_window() else { return };
    if ptr.is_null() {
        return;
    }
    let ns: &NSWindow = unsafe { &*(ptr as *const NSWindow) };
    // Close = 0, Miniaturize = 1, Zoom = 2.
    for b in [NSWindowButton(0), NSWindowButton(1), NSWindowButton(2)] {
        let Some(btn) = ns.standardWindowButton(b) else {
            continue;
        };
        let Some(sv) = (unsafe { btn.superview() }) else {
            continue;
        };
        let container_h = sv.frame().size.height;
        let f = btn.frame();
        // AppKit measures y from the container's bottom; the container's top is the
        // window top, so center the button within HEADER_H measured from the top.
        let top = (HEADER_H - f.size.height) / 2.0;
        let y = container_h - top - f.size.height;
        btn.setFrameOrigin(NSPoint::new(f.origin.x, y));
    }
}

// Center the traffic lights now, again after layout settles, and on every resize
// (macOS resets their positions on live-resize + fullscreen transitions).
#[cfg(target_os = "macos")]
fn arm_traffic_light_centering(window: &tauri::WebviewWindow) {
    center_traffic_lights(window);
    let w = window.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        let w2 = w.clone();
        let _ = w.run_on_main_thread(move || center_traffic_lights(&w2));
    });
    let w3 = window.clone();
    window.on_window_event(move |e| {
        if matches!(e, tauri::WindowEvent::Resized(_)) {
            center_traffic_lights(&w3);
        }
    });
}

// Bring the main window to the front (tray left-click / "Open Vellum" / a second
// launch). Restores the Dock icon (we may be a window-less menu-bar agent) and
// re-creates the window if a previous close destroyed it.
#[cfg(desktop)]
fn show_main_window(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    } else if let Err(e) = create_main_window(app) {
        tracing::error!("failed to re-create main window: {e}");
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
