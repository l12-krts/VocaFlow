use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Low,
    Medium,
    High,
    Ultra,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePref {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub onboarded: bool,
    pub hotkey: String,
    pub quality: Quality,
    pub language: String,
    pub preferred_languages: Vec<String>,
    pub formatting: bool,
    pub theme: ThemePref,
    pub sounds: bool,
    pub mic_id: Option<String>,
    pub launch_at_login: bool,
    pub typing_apps: Vec<String>,
    pub dictionary: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            onboarded: false,
            hotkey: "Alt+D".into(),
            quality: Quality::Medium,
            language: "auto".into(),
            preferred_languages: vec![],
            formatting: true,
            theme: ThemePref::System,
            sounds: true,
            mic_id: None,
            launch_at_login: true,
            typing_apps: vec![],
            dictionary: vec![],
        }
    }
}

pub struct Store {
    path: PathBuf,
    pub value: Settings,
}

impl Store {
    pub fn load(dir: PathBuf) -> Self {
        let path = dir.join("settings.json");
        let value = fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
            .unwrap_or_default();
        Self { path, value }
    }

    /// Merge a partial JSON object onto the current settings.
    pub fn patch(&mut self, patch: serde_json::Value) -> anyhow::Result<Settings> {
        let mut cur = serde_json::to_value(&self.value)?;
        if let (Some(cur), Some(p)) = (cur.as_object_mut(), patch.as_object()) {
            for (k, v) in p {
                cur.insert(k.clone(), v.clone());
            }
        }
        let mut next: Settings = serde_json::from_value(cur)?;
        next.dictionary = dedupe(next.dictionary);
        next.preferred_languages = dedupe(next.preferred_languages);
        self.value = next;
        self.save()?;
        Ok(self.value.clone())
    }

    fn save(&self) -> anyhow::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        // Write-then-rename so a crash never leaves a half-written file.
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(&self.value)?)?;
        fs::rename(tmp, &self.path)?;
        Ok(())
    }
}

fn dedupe(v: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(v.len());
    for s in v {
        let s = s.trim().to_string();
        if !s.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(&s)) {
            out.push(s);
        }
    }
    out
}
