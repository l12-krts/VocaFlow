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
    // whisper.cpp links Vulkan on Windows/Linux; let the linker find the
    // SDK's copy when the system has none.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        if let Ok(sdk) = std::env::var("VULKAN_SDK") {
            println!("cargo:rustc-link-search=native={sdk}/lib");
        }
    }
    tauri_build::build()
}
