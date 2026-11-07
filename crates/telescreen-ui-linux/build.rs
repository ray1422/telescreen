//! Compiles the cxx-qt bridge and the QML module `dev.telescreen.ui`. Linux only.

fn main() {
    #[cfg(target_os = "linux")]
    linux();
}

#[cfg(target_os = "linux")]
fn linux() {
    use cxx_qt_build::{CxxQtBuilder, QmlModule};

    let qml: Vec<String> = [
        "App",
        "MainPage",
        "G2Rect",
        "Card",
        "SectionHeader",
        "DisplaySection",
        "CandidateRow",
        "SessionCard",
        "TextRow",
        "SwitchRow",
    ]
    .iter()
    .map(|name| format!("qml/{name}.qml"))
    .collect();
    let builder = CxxQtBuilder::new_qml_module(QmlModule::new("dev.telescreen.ui").qml_files(qml))
        .qt_module("Gui")
        .qt_module("Quick")
        .qt_module("QuickControls2")
        .qt_module("Widgets")
        // In the module directory, next to the QML files.
        .qrc_resources(["qml/telescreen.png"])
        .file("src/bridge.rs")
        .cpp_file("src/app.cpp");
    // SAFETY: the closure only adds an include path.
    let builder = unsafe { builder.cc_builder(|cc| _ = cc.include("src")) };
    builder.build();
}
