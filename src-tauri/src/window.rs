use tauri::{AppHandle, Manager, WindowEvent};

pub fn show_and_focus_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    } else {
        eprintln!("Hauptfenster nicht gefunden — konnte es nicht anzeigen");
    }
}

pub fn install_hide_on_close(app: &AppHandle) {
    let Some(main_window) = app.get_webview_window("main") else {
        eprintln!("Hauptfenster nicht gefunden — Schließen-Verhalten nicht verdrahtet");
        return;
    };
    let window_to_hide = main_window.clone();
    main_window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = window_to_hide.hide();
        }
    });
}
