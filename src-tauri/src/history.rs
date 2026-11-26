use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const KEEP: usize = 50;

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub text: String,
    pub raw: String,
    pub app: Option<String>,
    pub format: Option<String>,
    /// Unix millis.
    pub at: u64,
    pub duration_ms: u64,
}

pub struct History {
    path: PathBuf,
    pub items: Vec<Item>,
}

impl History {
    pub fn load(dir: PathBuf) -> Self {
        let path = dir.join("history.json");
        let items = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { path, items }
    }

    pub fn push(&mut self, item: Item) {
        self.items.insert(0, item);
        self.items.truncate(KEEP);
        self.save();
    }

    pub fn delete(&mut self, id: &str) {
        self.items.retain(|i| i.id != id);
        self.save();
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.save();
    }

    fn save(&self) {
        if let Some(d) = self.path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_vec(&self.items).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(tmp, &self.path);
        }
    }
}
