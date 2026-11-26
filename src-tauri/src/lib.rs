mod audio;
mod cleanup;
mod commands;
mod context;
mod core;
mod formatter;
mod history;
mod hotkey;
mod insert;
mod models;
mod platform;
mod settings;
mod stt;
mod ui;

use crate::core::Core;
use std::sync::Arc;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Manager;
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,whisper_rs=warn")).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| ui::show_main(app, None)))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--background"])))
        .plugin(hotkey::plugin())
        .invoke_handler(tauri::generate_handler![
            commands::platform,
            commands::get_settings,
            commands::set_settings,
            commands::models_state,
            commands::retry_download,
            commands::list_mics,
            commands::mic_meter,
            commands::history,
            commands::clear_history,
            commands::delete_history,
            commands::copy_text,
            commands::permissions,
            commands::request_permission,
            commands::capture_hotkey,
            commands::pill_ready,
            commands::pill_shape,
            commands::main_ready,
            commands::hide_main,
            commands::quit,
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            unsafe {
                platform::macos::set_accessory();
            }
            let handle = app.handle().clone();
            let config = app.path().app_config_dir()?;
            let data = app.path().app_data_dir()?;
            let core = Core::new(handle.clone(), config, data);
            app.manage(core.clone());

            let st = core.settings();
            if let Err(e) = hotkey::register(&handle, &st.hotkey) {
                log::error!("hotkey {}: {e:#}", st.hotkey);
            }
            sync_autostart(&handle, st.launch_at_login);
            #[cfg(target_os = "linux")]
            platform::linux::start_focus_tracking();

            ui::create_overlays(&handle)?;
            build_tray(&handle)?;

            // First run goes straight to onboarding; later launches (e.g.
            // at login) stay quietly in the tray.
            let background = std::env::args().any(|a| a == "--background");
            ui::create_main(&handle, !st.onboarded)?;
            if !st.onboarded || !background {
                ui::show_main(&handle, None);
            }
            core.sync_models();
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building VocaFlow")
        .run(|_app, event| {
            // Stay alive in the tray when Settings closes.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}

pub(crate) fn sync_autostart(app: &tauri::AppHandle, on: bool) {
    let al = app.autolaunch();
    let cur = al.is_enabled().unwrap_or(false);
    if on && !cur {
        let _ = al.enable();
    } else if !on && cur {
        let _ = al.disable();
    }
}

fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Settings…", true, None::<&str>)?;
    let history = MenuItem::with_id(app, "history", "History", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit VocaFlow", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &history, &PredefinedMenuItem::separator(app)?, &quit])?;
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;
    TrayIconBuilder::with_id("main")
        .icon(icon)
        .icon_as_template(true)
        .tooltip("VocaFlow")
        .menu(&menu)
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_menu_event(|app, e| match e.id().as_ref() {
            "open" => ui::show_main(app, Some("general")),
            "history" => ui::show_main(app, Some("history")),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                if !cfg!(target_os = "macos") {
                    ui::show_main(tray.app_handle(), None);
                }
            }
        })
        .build(app)?;
    Ok(())
}

#[allow(dead_code)]
fn _assert_send(_: Arc<Core>) {}

/// Headless end-to-end check of the engine (download, transcribe,
/// format) used by `cargo run --example e2e`. Not used by the app.
#[doc(hidden)]
pub mod e2e {
    use crate::context::{classify, Context};
    use crate::{audio, cleanup, formatter, models, settings::Quality, stt};
    use std::time::{Duration, Instant};

    pub fn run(wav: &str, dir: &str, quality: &str, contexts: &[(&str, &str, &str)], format: bool) -> anyhow::Result<()> {
        let q: Quality = serde_json::from_value(serde_json::Value::String(quality.into()))?;
        let m = models::Models::new(dir.into());
        let rt = tokio::runtime::Runtime::new()?;
        let t = Instant::now();
        rt.block_on(m.ensure_speech(q, |s| eprint!("\r{:?}            ", s.speech)));
        eprintln!("\nspeech model ready in {:?}", t.elapsed());
        if format {
            rt.block_on(m.ensure_formatter(|s| eprint!("\r{:?}            ", s.formatter)));
            eprintln!("\nformatter ready");
        }
        let samples = read_wav(wav)?;
        let audio = audio::prepare(samples).ok_or_else(|| anyhow::anyhow!("no speech"))?;
        let eng = stt::Engine::new();
        let path = m.speech_path(q);
        let t = Instant::now();
        eng.warm(&path)?;
        eprintln!("stt load {:?}", t.elapsed());
        let t = Instant::now();
        let tr = eng.transcribe(&path, &audio, &stt::Request { language: "auto", preferred: &["en".into(), "el".into()], dictionary: &["VocaFlow".into()], quality: q })?;
        println!("RAW [{}] ({:?}): {}", tr.language, t.elapsed(), tr.text);
        let f = formatter::Formatter::new();
        if format {
            let t = Instant::now();
            f.warm(&m.path(models::FORMATTER[0].file), &m.path(models::FORMATTER[1].file))?;
            eprintln!("formatter load {:?}", t.elapsed());
        }
        for (app, title, before) in contexts {
            let ctx = Context { app: app.to_string(), title: title.to_string(), before: before.to_string(), ..Default::default() };
            let kind = classify(&ctx);
            let basic = cleanup::basic(&tr.text, kind);
            println!("\n== {app} / {title} -> {kind:?}\nRULES: {basic}");
            if format {
                let t = Instant::now();
                let out = f.format(&cleanup::spoken_punctuation(&tr.text), kind, &ctx, Duration::from_secs(60))?;
                println!("MODEL ({:?}):\n{}", t.elapsed(), out.unwrap_or_else(|| "<rejected>".into()));
            }
        }
        Ok(())
    }

    fn read_wav(p: &str) -> anyhow::Result<Vec<f32>> {
        let b = std::fs::read(p)?;
        let rate = u32::from_le_bytes(b[24..28].try_into()?);
        let ch = u16::from_le_bytes(b[22..24].try_into()?) as usize;
        let mut i = 12;
        while i + 8 < b.len() {
            let id = &b[i..i + 4];
            let len = u32::from_le_bytes(b[i + 4..i + 8].try_into()?) as usize;
            if id == b"data" {
                let d = &b[i + 8..(i + 8 + len).min(b.len())];
                let s: Vec<f32> = d.chunks_exact(2 * ch).map(|f| i16::from_le_bytes([f[0], f[1]]) as f32 / 32768.0).collect();
                return Ok(audio::resample(&s, rate, audio::WHISPER_RATE));
            }
            i += 8 + len;
        }
        anyhow::bail!("no data chunk")
    }
}
