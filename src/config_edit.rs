//! Cambios a la config desde la UI. Usa `toml_edit` para conservar comentarios,
//! orden y formato del archivo, y valida el resultado antes de escribirlo.

use std::path::Path;

use anyhow::{Context, Result, bail};
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, Value, value};

use crate::{checks, config::Config};

/// Claves de `[[service]]` que edita el formulario, en el orden en que se escriben.
pub const SERVICE_KEYS: [&str; 13] = [
    "name",
    "kind",
    "target",
    "unit",
    "command",
    "host",
    "suffix",
    "link",
    "expect_status",
    "interval",
    "timeout",
    "slow_ms",
    "mute",
];

/// Valores de un servicio como texto, tal como salen del formulario.
/// Una clave vacía o ausente se elimina del archivo.
pub type Values = Vec<(&'static str, String)>;

pub fn read(path: &Path) -> Result<DocumentMut> {
    let raw = std::fs::read_to_string(path)?;
    Ok(raw.parse::<DocumentMut>()?)
}

/// Valida a fondo (incluido construir cada check) y escribe de forma atómica.
pub fn save(path: &Path, doc: &DocumentMut) -> Result<()> {
    let text = doc.to_string();
    validate(&text)?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn validate(text: &str) -> Result<()> {
    let cfg = Config::parse(text)?;
    for s in &cfg.services {
        checks::build(s, &cfg)?;
    }
    Ok(())
}

fn services_mut(doc: &mut DocumentMut) -> &mut ArrayOfTables {
    if doc
        .get("service")
        .and_then(Item::as_array_of_tables)
        .is_none()
    {
        doc.insert("service", Item::ArrayOfTables(ArrayOfTables::new()));
    }
    doc["service"]
        .as_array_of_tables_mut()
        .expect("recién insertado")
}

fn find_service(doc: &mut DocumentMut, name: &str) -> Option<usize> {
    services_mut(doc)
        .iter()
        .position(|t| t.get("name").and_then(Item::as_str) == Some(name))
}

/// Escribe los valores en la tabla: números y booleanos con su tipo, el resto texto.
fn apply(t: &mut Table, values: &Values) -> Result<()> {
    for (key, raw) in values {
        let raw = raw.trim();
        // `mute = false` es el valor por defecto: no hace falta escribirlo.
        if raw.is_empty() || (*key == "mute" && raw != "sí") {
            t.remove(key);
            continue;
        }
        let mut new: Value = match *key {
            "expect_status" | "slow_ms" => raw
                .parse::<i64>()
                .with_context(|| format!("`{key}` debe ser un número"))?
                .into(),
            "mute" => (raw == "sí").into(),
            _ => raw.into(),
        };
        // Conserva espacios y comentarios de la línea (`name = "x"  # nota`).
        if let Some(old) = t.get(key).and_then(Item::as_value) {
            *new.decor_mut() = old.decor().clone();
        }
        t[key] = Item::Value(new);
    }
    Ok(())
}

pub fn add_service(doc: &mut DocumentMut, values: &Values) -> Result<()> {
    let mut t = Table::new();
    apply(&mut t, values)?;
    services_mut(doc).push(t);
    Ok(())
}

/// Actualiza el servicio `old`; si cambia de nombre, también sus referencias en paneles.
pub fn update_service(doc: &mut DocumentMut, old: &str, values: &Values) -> Result<()> {
    let Some(i) = find_service(doc, old) else {
        bail!("no encuentro el servicio {old} en la config");
    };
    apply(services_mut(doc).get_mut(i).expect("índice válido"), values)?;
    let new = values
        .iter()
        .find(|(k, _)| *k == "name")
        .map(|(_, v)| v.trim().to_string())
        .unwrap_or_else(|| old.to_string());
    if new != old {
        for_each_panel_services(doc, |arr| {
            for v in arr.iter_mut() {
                if v.as_str() == Some(old) {
                    *v = new.as_str().into();
                }
            }
        });
    }
    Ok(())
}

/// Borra el servicio y lo quita de las listas `services` de los paneles.
pub fn delete_service(doc: &mut DocumentMut, name: &str) -> Result<()> {
    let Some(i) = find_service(doc, name) else {
        bail!("no encuentro el servicio {name} en la config");
    };
    services_mut(doc).remove(i);
    for_each_panel_services(doc, |arr| arr.retain(|v| v.as_str() != Some(name)));
    Ok(())
}

fn for_each_panel_services(doc: &mut DocumentMut, mut f: impl FnMut(&mut Array)) {
    let Some(pages) = doc.get_mut("page").and_then(Item::as_array_of_tables_mut) else {
        return;
    };
    for page in pages.iter_mut() {
        let Some(panels) = page.get_mut("panel").and_then(Item::as_array_of_tables_mut) else {
            continue;
        };
        for panel in panels.iter_mut() {
            if let Some(arr) = panel.get_mut("services").and_then(Item::as_array_mut) {
                f(arr);
            }
        }
    }
}

/// Sin `[[page]]`, la app muestra una página por defecto que no está en el
/// archivo; antes de editar páginas se escribe explícitamente.
fn pages_mut(doc: &mut DocumentMut) -> &mut ArrayOfTables {
    let empty = doc
        .get("page")
        .and_then(Item::as_array_of_tables)
        .is_none_or(ArrayOfTables::is_empty);
    if empty {
        let mut panels = ArrayOfTables::new();
        for kind in ["status", "chart"] {
            let mut t = Table::new();
            t["kind"] = value(kind);
            panels.push(t);
        }
        let mut page = Table::new();
        page["title"] = value("Overview");
        page.insert("panel", Item::ArrayOfTables(panels));
        let mut pages = ArrayOfTables::new();
        pages.push(page);
        doc.insert("page", Item::ArrayOfTables(pages));
    }
    doc["page"]
        .as_array_of_tables_mut()
        .expect("recién insertado")
}

pub fn add_page(doc: &mut DocumentMut, title: &str) {
    let mut t = Table::new();
    t["title"] = value(title.trim());
    pages_mut(doc).push(t);
}

pub struct NewPanel {
    pub kind: String,
    pub title: String,
    pub services: Vec<String>,
    pub window: String,
}

pub fn add_panel(doc: &mut DocumentMut, page: usize, p: &NewPanel) -> Result<()> {
    let Some(page) = pages_mut(doc).get_mut(page) else {
        bail!("la página no existe en la config");
    };
    if page
        .get("panel")
        .and_then(Item::as_array_of_tables)
        .is_none()
    {
        page.insert("panel", Item::ArrayOfTables(ArrayOfTables::new()));
    }
    let mut t = Table::new();
    t["kind"] = value(&p.kind);
    if !p.title.trim().is_empty() {
        t["title"] = value(p.title.trim());
    }
    if !p.services.is_empty() {
        t["services"] = value(p.services.iter().map(String::as_str).collect::<Array>());
    }
    if !p.window.is_empty() {
        t["window"] = value(&p.window);
    }
    page["panel"]
        .as_array_of_tables_mut()
        .expect("recién insertado")
        .push(t);
    Ok(())
}

pub fn remove_panel(doc: &mut DocumentMut, page: usize, panel: usize) -> Result<()> {
    let panels = pages_mut(doc)
        .get_mut(page)
        .and_then(|p| p.get_mut("panel"))
        .and_then(Item::as_array_of_tables_mut);
    match panels {
        Some(panels) if panel < panels.len() => {
            panels.remove(panel);
            Ok(())
        }
        _ => bail!("el panel no existe en la config"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r#"# mi config
[[service]]
name = "web"   # la principal
kind = "http"
target = "https://example.com"

[[service]]
name = "db"
kind = "tcp"
target = "localhost:5432"

[[page]]
title = "Todo"
  [[page.panel]]
  kind = "chart"
  services = ["web", "db"]
"#;

    fn vals(pairs: &[(&'static str, &str)]) -> Values {
        pairs.iter().map(|(k, v)| (*k, v.to_string())).collect()
    }

    #[test]
    fn rename_keeps_comments_and_updates_panels() {
        let mut doc: DocumentMut = DOC.parse().unwrap();
        update_service(
            &mut doc,
            "web",
            &vals(&[("name", "sitio"), ("slow_ms", "800"), ("link", "")]),
        )
        .unwrap();
        let out = doc.to_string();
        assert!(out.contains("# mi config"));
        assert!(out.contains("# la principal"));
        assert!(out.contains(r#"name = "sitio""#));
        assert!(out.contains("slow_ms = 800"));
        assert!(out.contains(r#"services = ["sitio", "db"]"#));
        validate(&out).unwrap();
    }

    #[test]
    fn delete_removes_references() {
        let mut doc: DocumentMut = DOC.parse().unwrap();
        delete_service(&mut doc, "db").unwrap();
        let out = doc.to_string();
        assert!(!out.contains("5432"));
        assert!(out.contains(r#"services = ["web"]"#));
        validate(&out).unwrap();
    }

    #[test]
    fn add_service_panel_and_page() {
        let mut doc: DocumentMut = DOC.parse().unwrap();
        add_service(
            &mut doc,
            &vals(&[
                ("name", "dns"),
                ("kind", "ping"),
                ("target", "1.1.1.1"),
                ("mute", "sí"),
            ]),
        )
        .unwrap();
        add_page(&mut doc, "Red");
        add_panel(
            &mut doc,
            1,
            &NewPanel {
                kind: "spark".into(),
                title: String::new(),
                services: vec!["dns".into()],
                window: String::new(),
            },
        )
        .unwrap();
        let out = doc.to_string();
        assert!(out.contains("mute = true"));
        let cfg = Config::parse(&out).unwrap();
        assert_eq!(cfg.pages[1].panels[0].services, ["dns"]);
        remove_panel(&mut doc, 0, 0).unwrap();
        assert!(
            Config::parse(&doc.to_string()).unwrap().pages[0]
                .panels
                .is_empty()
        );
    }

    #[test]
    fn default_page_is_materialized() {
        let mut doc: DocumentMut = "[[service]]\nname = \"a\"\nkind = \"tcp\"\ntarget = \"x:1\"\n"
            .parse()
            .unwrap();
        add_page(&mut doc, "Nueva");
        let cfg = Config::parse(&doc.to_string()).unwrap();
        assert_eq!(cfg.pages.len(), 2);
        assert_eq!(cfg.pages[0].title, "Overview");
        assert_eq!(cfg.pages[0].panels.len(), 2);
    }

    #[test]
    fn invalid_result_is_rejected() {
        let mut doc: DocumentMut = DOC.parse().unwrap();
        // Un http sin target no se puede construir.
        add_service(&mut doc, &vals(&[("name", "x"), ("kind", "http")])).unwrap();
        assert!(validate(&doc.to_string()).is_err());
        // Nombre duplicado.
        let mut doc: DocumentMut = DOC.parse().unwrap();
        add_service(
            &mut doc,
            &vals(&[("name", "db"), ("kind", "tcp"), ("target", "a:1")]),
        )
        .unwrap();
        assert!(validate(&doc.to_string()).is_err());
    }
}
