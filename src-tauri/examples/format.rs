//! cargo run --example format -- <text file> <models dir> <app> <window title> <lang>
fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,whisper_rs=warn")).init();
    let a: Vec<String> = std::env::args().collect();
    vocaflow_lib::e2e::format_file(&a[1], &a[2], &a[3], &a[4], &a[5]).unwrap();
}
