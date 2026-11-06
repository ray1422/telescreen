use std::process::Command;

fn query(var: &str) -> String {
    let out = Command::new("qmake6").args(["-query", var]).output().expect("qmake6");
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn main() {
    // The private QWaylandWindow native interface is in the versioned private headers.
    let headers = query("QT_INSTALL_HEADERS");
    let version = query("QT_VERSION");
    let builder = cxx_qt_build::CxxQtBuilder::new()
        .qt_module("Gui")
        .qt_module("Qml")
        .qt_module("Quick")
        .file("src/main.rs")
        .cpp_file("src/shim.cpp");
    let builder = unsafe {
        builder.cc_builder(|cc| {
            cc.include("src");
            for module in ["QtCore", "QtGui"] {
                cc.include(format!("{headers}/{module}/{version}"));
                cc.include(format!("{headers}/{module}/{version}/{module}"));
            }
        })
    };
    builder.build();
}
