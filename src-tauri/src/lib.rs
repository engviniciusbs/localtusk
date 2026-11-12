mod apitools;
mod backup;
mod blueprint;
mod commands;
mod download;
mod extras;
mod mysql_manager;
mod php_manager;
mod project;
mod runtime;
mod tools;
mod wp;

use commands::AppState;
use tauri::RunEvent;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = AppState::new();

    // Se a sessão anterior foi morta à força, os serviços dos projetos podem ter ficado
    // rodando. O retrato é tirado agora (antes de qualquer start) e o encerramento
    // acontece em segundo plano, sem atingir o que for iniciado depois.
    {
        let orphans: Vec<(i32, String)> = state
            .projects
            .lock()
            .unwrap()
            .iter()
            .flat_map(|p| runtime::project_processes(&state.base, &p.id))
            .collect();
        if !orphans.is_empty() {
            std::thread::spawn(move || {
                runtime::kill_processes(&orphans);
            });
        }
    }

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![
            commands::list_projects,
            commands::check_tools,
            commands::list_php_versions,
            commands::create_project,
            commands::start_project,
            commands::stop_project,
            commands::delete_project,
            commands::set_php_version,
            commands::open_site,
            commands::open_folder,
            commands::read_log,
            commands::app_info,
            commands::open_data_dir,
            commands::php_catalog,
            commands::install_php,
            commands::uninstall_php,
            commands::extras_status,
            commands::install_extra,
            commands::uninstall_extra,
            commands::open_tool,
            commands::open_terminal,
            commands::install_wordpress,
            commands::read_requests,
            commands::clear_requests,
            commands::load_collection,
            commands::save_collection,
            commands::export_collection,
            commands::read_collection_file,
            commands::send_request,
            commands::list_backups,
            commands::create_backup,
            commands::restore_backup,
            commands::delete_backup,
            commands::open_backups_dir,
            commands::list_blueprints,
            commands::create_blueprint,
            commands::create_project_from_blueprint,
            commands::delete_blueprint,
            commands::import_blueprint,
            commands::export_blueprint,
            commands::open_blueprints_dir,
            commands::list_mysql_versions,
            commands::mysql_catalog,
            commands::install_mysql,
            commands::uninstall_mysql,
            commands::set_db_version,
        ])
        .build(tauri::generate_context!())
        .expect("erro ao iniciar o LocalTusk");

    app.run(move |_handle, event| {
        // Ao fechar o app, encerra todos os serviços para não deixar processos órfãos.
        if let RunEvent::Exit = event {
            state.stop_all();
        }
    });
}
