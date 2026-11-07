//! Shows the main window with a `MainScreen` fixture, and prints each UI event.
//! The window reloads the fixture when the file changes.
//!
//!     cargo run -p telescreen-ui-linux --example preview -- crates/telescreen-ui-linux/fixtures/main-idle.json

#[cfg(target_os = "linux")]
fn main() {
    use std::time::{Duration, SystemTime};

    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: preview <MainScreen JSON file>");
        std::process::exit(2);
    };
    let load = move || -> Option<(SystemTime, telescreen_view::MainScreen)> {
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
        let text = std::fs::read_to_string(&path).ok()?;
        match serde_json::from_str(&text) {
            Ok(screen) => Some((modified, screen)),
            Err(e) => {
                eprintln!("preview: {path}: {e}");
                None
            }
        }
    };
    let ui = telescreen_ui_linux::LinuxUi::new(|event| {
        println!("preview: {event:?}");
        // UI-15: the app stops only for "Quit Telescreen".
        if event == telescreen_ui_linux::UiEvent::Quit {
            telescreen_ui_linux::quit();
        }
    });
    std::thread::spawn(move || {
        let mut last = None;
        loop {
            if let Some((modified, screen)) = load()
                && last != Some(modified)
            {
                last = Some(modified);
                if let Err(e) = ui.push_main(&screen) {
                    eprintln!("preview: {e}");
                }
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    });
    std::process::exit(telescreen_ui_linux::run());
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("preview: Linux only");
}
