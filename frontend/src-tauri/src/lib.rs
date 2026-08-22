mod runtime;

use std::sync::Mutex;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let runtime = runtime::DesktopRuntime::start(app.handle())?;
            app.manage(Mutex::new(runtime));
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build NovelWorld desktop runtime");

    app.run(|app, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            if let Some(runtime) = app.try_state::<Mutex<runtime::DesktopRuntime>>() {
                if let Ok(mut runtime) = runtime.lock() {
                    runtime.shutdown();
                }
            }
        }
    });
}
