// Build script for the cxx-qt app.
//
// CXX-Qt parses the `#[cxx_qt::bridge]` in `src/main.rs`, generates the C++ QObject + MOC code,
// and compiles + links it together with Qt. The `Backend` QObject (annotated `#[qml_element]`) is
// registered into a QML module so the QML can `import` it and instantiate `Backend {}`. The
// `qml/main.qml` file is baked into the binary's Qt resources under
// `qrc:/qt/qml/com/example/gitreview/qml/main.qml`.
use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    CxxQtBuilder::new()
        // QML runtime modules the UI imports (QtQuick, Controls, Layouts).
        .qt_module("Qml")
        .qt_module("Quick")
        .qt_module("QuickControls2")
        // Qt Gui is needed for the QPalette-based light/dark detection in cpp/theme.cpp.
        .qt_module("Gui")
        .qml_module(QmlModule {
            uri: "com.example.gitreview",
            version_major: 1,
            version_minor: 0,
            rust_files: &["src/main.rs"],
            qml_files: &["qml/main.qml"],
            ..Default::default()
        })
        // Compile the small theme-detection helper alongside the generated C++.
        .cc_builder(|cc| {
            cc.file("cpp/theme.cpp");
        })
        .build();

    println!("cargo:rerun-if-changed=cpp/theme.cpp");
}
