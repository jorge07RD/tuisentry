//! Estado que sobrevive reinicios, en `~/.local/state/tuisentry/`.
//! Fallar al leer o escribir nunca es fatal: se usan los valores por defecto.

use std::path::PathBuf;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::view::View;

pub fn state_dir() -> PathBuf {
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("tuisentry")
}

fn load<T: DeserializeOwned + Default>(name: &str) -> T {
    std::fs::read(state_dir().join(name))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save<T: Serialize>(name: &str, value: &T) {
    let dir = state_dir();
    let Ok(json) = serde_json::to_vec_pretty(value) else {
        return;
    };
    let tmp = dir.join(format!("{name}.tmp"));
    // Escritura atómica: un corte a mitad no deja el archivo roto.
    let _ = std::fs::create_dir_all(&dir)
        .and_then(|_| std::fs::write(&tmp, json))
        .and_then(|_| std::fs::rename(&tmp, dir.join(name)));
}

/// Preferencias de la interfaz.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct UiPrefs {
    pub theme: Option<String>,
    pub view: Option<View>,
}

impl UiPrefs {
    pub fn load() -> Self {
        load("ui.json")
    }

    pub fn save(&self) {
        save("ui.json", self)
    }
}

/// Servicios pausados o silenciados, por nombre.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct EnginePrefs {
    pub paused: Vec<String>,
    pub muted: Vec<String>,
}

impl EnginePrefs {
    pub fn load() -> Self {
        load("engine.json")
    }

    pub fn save(&self) {
        save("engine.json", self)
    }
}
