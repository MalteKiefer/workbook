use tauri::{AppHandle, Manager, WindowEvent};

pub fn show_and_focus(window: &tauri::WebviewWindow) {
    let _ = window.show();
    let _ = window.set_focus();
}

pub fn show_and_focus_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        show_and_focus(&window);
    } else {
        eprintln!("Hauptfenster nicht gefunden — konnte es nicht anzeigen");
    }
}

pub fn install_hide_on_close_for(window: &tauri::WebviewWindow) {
    let window_to_hide = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = window_to_hide.hide();
        }
    });
}

pub fn install_hide_on_close(app: &AppHandle) {
    match app.get_webview_window("main") {
        Some(window) => install_hide_on_close_for(&window),
        None => eprintln!("Hauptfenster nicht gefunden — Schließen-Verhalten nicht verdrahtet"),
    }
}
