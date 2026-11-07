//! The preview fixtures have the JSON shape of `MainScreen` (UI-18). The QML and
//! the applet read this shape, so a fixture that does not parse is out of date.
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use telescreen_view::MainScreen;

#[test]
fn each_fixture_is_a_main_screen() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let mut count = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|x| x == "json") {
            let text = std::fs::read_to_string(&path).unwrap();
            let screen: Result<MainScreen, _> = serde_json::from_str(&text);
            assert!(screen.is_ok(), "{}: {:?}", path.display(), screen.err());
            count += 1;
        }
    }
    assert!(count > 0);
}

/// The applet package must be complete for `kpackagetool6`, so it has copies of
/// the G2 shape and the glyphs; the QML module has a copy of the icon. The
/// copies must be the same as their sources.
#[test]
fn the_applet_copies_are_current() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let applet = root.join("applet/dev.telescreen.menu/contents");
    let mut pairs = vec![
        (
            root.join("crates/telescreen-ui-linux/qml/G2Rect.qml"),
            applet.join("ui/G2Rect.qml"),
        ),
        (
            root.join("assets/linux/telescreen.png"),
            root.join("crates/telescreen-ui-linux/qml/telescreen.png"),
        ),
    ];
    for state in ["idle", "menu-open", "controlling", "controlled"] {
        let name = format!("glyph-{state}.svg");
        pairs.push((
            root.join("assets/linux").join(&name),
            applet.join("icons").join(&name),
        ));
    }
    for (source, copy) in pairs {
        assert_eq!(
            std::fs::read(&source).unwrap(),
            std::fs::read(&copy).unwrap(),
            "copy {} to {}",
            source.display(),
            copy.display()
        );
    }
}
