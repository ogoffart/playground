# git-review — cxx-qt edition

A GitHub-style git code-review desktop app, built with **[cxx-qt](https://github.com/KDAB/cxx-qt)**
(KDAB's Qt ⇄ Rust bridge) and a Qt Quick / QML front-end. Part of a multi-framework comparison; the
behaviour and visuals follow the shared [`../SPEC.md`](../SPEC.md).

The Rust side opens the repository with `git2`, builds the diff model, highlights it with `syntect`,
serializes it to JSON strings, and exposes those (plus the toolbar settings) on a `Backend` QObject.
The QML in [`qml/main.qml`](qml/main.qml) renders it with a resizable `SplitView`, a `ListView`
whose `section` gives true sticky per-file headers, and `Text { textFormat: RichText }` for the
syntax-highlighted diff lines. The git/diff/highlight model (`src/git.rs`, `src/highlight.rs`) is the
shared, framework-agnostic code reused verbatim across every app in the repo.

## Build & run

Qt6 is located via `qmake`. This crate ships a [`.cargo/config.toml`](.cargo/config.toml) that points
`QMAKE`/`QMAKE6` at `/usr/bin/qmake6`, so a plain build works:

```sh
cd qt-cxx
cargo build
./target/debug/git-review-cxxqt /path/to/a/git/repo
```

The repo to review is taken from `argv[1]`, else `$GIT_REVIEW_REPO`, else the current directory.
Generate a demo repo with `../tools/make-sample-repo.sh`.

On a headless box, run under Xvfb with the xcb platform:

```sh
export DISPLAY=:86 QT_QPA_PLATFORM=xcb LIBGL_ALWAYS_SOFTWARE=1
Xvfb :86 -screen 0 1280x860x24 &
./target/debug/git-review-cxxqt /tmp/git-review-sample
```

## How cxx-qt differs from qmetaobject-rs

Both apps share the *exact same* QML and the same JSON-over-the-bridge model, so they're a clean
A/B of the two Rust→Qt binding strategies. The differences are all on the Rust side:

- **Bridge macro vs. derive macro.** qmetaobject-rs uses a `#[derive(QObject)]` struct with
  `qt_property!` / `qt_method!` / `qt_signal!` *field macros* that expand inline. cxx-qt instead uses
  a `#[cxx_qt::bridge]` *module*: the QObject is a `type Backend = super::BackendRust;` inside an
  `extern "RustQt"` block, properties are `#[qproperty(T, name)]` attributes on that type, and slots
  are `#[qinvokable]` function signatures. The Rust state lives in a separate plain `BackendRust`
  struct; the generated `Backend` wraps it.

- **Generated C++ + real MOC, built by a build script.** cxx-qt is built on **CXX**: `build.rs`
  runs `CxxQtBuilder`, which parses the bridge, *generates a C++ QObject subclass*, runs Qt's **moc**
  on it, compiles that with the `cc` crate, and links against Qt. qmetaobject-rs has no build step and
  no generated C++ — it registers a `QMetaObject` dynamically at runtime from the macro output.

- **CXX type safety at the FFI boundary.** Method/property/signal signatures cross into C++ through
  CXX's checked bindings, so only CXX-compatible types (`QString`, `i32`, `bool`, …) appear in the
  bridge. Inside Rust you reach the state via `self.rust()` / `self.as_mut().rust_mut()` and write
  properties through generated `set_*` setters that auto-emit the matching `*Changed` signal — versus
  qmetaobject-rs, where you mutate `self.field` directly and fire one hand-rolled `updated()` signal.

- **QML registration is a compile-time QML module.** The `Backend` is annotated `#[qml_element]` and
  registered into a QML module (`com.example.gitreview`) by `build.rs`; the QML file is baked into Qt
  resources at `qrc:/qt/qml/com/example/gitreview/qml/main.qml`. So QML does
  `import com.example.gitreview` and instantiates `Backend {}` as a first-class type. qmetaobject-rs
  instead injects the object as a *context property* (`engine.set_object_property("backend", …)`).
  Because cxx-qt's QML can't pass constructor arguments, the repo path is resolved in `main` and read
  by the QObject's `cxx_qt::Initialize::initialize()` hook, which opens the repo and builds the model.

- **App startup uses real Qt types.** `main` constructs a `QGuiApplication` and a
  `QQmlApplicationEngine` from `cxx-qt-lib` (thin Rust wrappers over the C++ classes) and calls
  `engine.load(...)` / `app.exec()` — close to idiomatic C++ Qt, whereas qmetaobject-rs exposes its
  own `QmlEngine` wrapper.
