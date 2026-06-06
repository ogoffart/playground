use std::process::Command;

fn pkgconfig(args: &[&str]) -> Vec<String> {
    let out = Command::new("pkg-config")
        .args(args)
        .output()
        .expect("failed to run pkg-config (is it installed?)");
    if !out.status.success() {
        panic!(
            "pkg-config {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .map(|s| s.to_string())
        .collect()
}

fn main() {
    println!("cargo:rerun-if-changed=src/shim.cpp");

    // Discover Qt6 Widgets via pkg-config.
    let cflags = pkgconfig(&["--cflags", "Qt6Widgets"]);
    let libs = pkgconfig(&["--libs", "Qt6Widgets"]);

    let mut build = cc::Build::new();
    build.cpp(true).std("c++17").file("src/shim.cpp");

    for f in &cflags {
        if let Some(inc) = f.strip_prefix("-I") {
            build.include(inc);
        } else if let Some(def) = f.strip_prefix("-D") {
            let mut parts = def.splitn(2, '=');
            let k = parts.next().unwrap();
            let v = parts.next();
            build.define(k, v);
        } else if f == "-fPIC" {
            build.pic(true);
        }
    }
    // Qt headers need this on g++.
    build.flag_if_supported("-fPIC");
    build.flag_if_supported("-Wno-unused-parameter");
    build.compile("qtshim");

    // Link flags from pkg-config: emit -L and -l in the form cargo expects.
    for l in &libs {
        if let Some(path) = l.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={}", path);
        } else if let Some(name) = l.strip_prefix("-l") {
            println!("cargo:rustc-link-lib=dylib={}", name);
        }
    }
    // Ensure the C++ standard library is linked.
    println!("cargo:rustc-link-lib=dylib=stdc++");
}
