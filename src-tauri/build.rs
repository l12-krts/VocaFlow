fn main() {
    // whisper.cpp's Metal backend uses Objective-C `@available`, which
    // compiles to `___isPlatformVersionAtLeast` from clang's runtime.
    // Rust links with `-nodefaultlibs`, so add it explicitly.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        if let Ok(out) = std::process::Command::new("xcrun").args(["clang", "--print-resource-dir"]).output() {
            let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !dir.is_empty() {
                println!("cargo:rustc-link-search=native={dir}/lib/darwin");
                println!("cargo:rustc-link-lib=static=clang_rt.osx");
            }
        }
    }
    tauri_build::build()
}
