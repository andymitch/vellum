mod vault;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // iroh's networking stack uses rustls with no built-in provider; install one
    // before anything spins up a TLS client (relay/discovery). Required on
    // Android, harmless if a provider is already set elsewhere.
    let _ = rustls::crypto::ring::default_provider().install_default();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let state = tauri::async_runtime::block_on(vault::init(dir))?;
            app.manage(state);
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
