use tauri::AppHandle;

use crate::{quickcapture, window};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliAction {
    QuickCapture,
    Search,
    None,
}

pub fn parse_args(args: &[String]) -> CliAction {
    if args.iter().any(|a| a == "--quick-capture") {
        CliAction::QuickCapture
    } else if args.iter().any(|a| a == "--search") {
        CliAction::Search
    } else {
        CliAction::None
    }
}

pub fn dispatch(app: &AppHandle, action: CliAction) {
    match action {
        CliAction::QuickCapture => {
            if let Err(e) = quickcapture::open(app) {
                eprintln!("Schnellerfassung (CLI) fehlgeschlagen: {e}");
            }
        }
        CliAction::Search => window::show_and_focus_main(app),
        CliAction::None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quick_capture_flag() {
        assert_eq!(
            parse_args(&["wartungsdoku".to_string(), "--quick-capture".to_string()]),
            CliAction::QuickCapture
        );
    }

    #[test]
    fn parses_search_flag() {
        assert_eq!(
            parse_args(&["wartungsdoku".to_string(), "--search".to_string()]),
            CliAction::Search
        );
    }

    #[test]
    fn no_recognized_flag_is_none() {
        assert_eq!(parse_args(&["wartungsdoku".to_string()]), CliAction::None);
    }
}
