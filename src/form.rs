//! Formularios táctiles para añadir y editar servicios, paneles y páginas.
//! Los campos dependen del tipo elegido (un `http` pide URL, un `systemd` pide
//! unidad y host…), y cambiar el tipo conserva lo ya escrito.

use std::collections::HashMap;

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::config::ServiceCfg;

pub const SERVICE_KINDS: [&str; 5] = ["http", "tcp", "ping", "systemd", "cmd"];
pub const PANEL_KINDS: [&str; 4] = ["status", "chart", "spark", "gauge"];
pub const WINDOWS: [&str; 6] = ["", "5m", "15m", "1h", "6h", "24h"];
/// Valor del selector de host que significa "en esta máquina".
pub const LOCAL: &str = "local";

#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    Text,
    Number,
    Select(Vec<String>),
    /// Varias opciones marcables; `value` las une con `\n`.
    Multi(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub kind: FieldKind,
    pub value: String,
    /// Posición del cursor en caracteres.
    pub cursor: usize,
}

impl Field {
    pub fn selected(&self) -> Vec<&str> {
        self.value.split('\n').filter(|s| !s.is_empty()).collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// `None` = servicio nuevo; `Some(nombre)` = editar ese.
    Service(Option<String>),
    /// Panel nuevo en la página indicada.
    Panel(usize),
    Page,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Continue,
    Save,
    Cancel,
}

#[derive(Debug)]
pub struct Form {
    pub title: String,
    pub target: Target,
    pub fields: Vec<Field>,
    pub focus: usize,
    pub error: Option<String>,
    pub scroll: usize,
    /// Opción resaltada en un campo `Multi` (teclado).
    pub opt: usize,
    stash: HashMap<&'static str, String>,
    /// Hosts de la config, para el selector de host.
    hosts: Vec<String>,
    /// Nombres de servicio, para el selector múltiple de paneles.
    services: Vec<String>,
}

impl Form {
    fn new(title: String, target: Target, stash: HashMap<&'static str, String>) -> Self {
        let mut f = Self {
            title,
            target,
            fields: vec![],
            focus: 0,
            error: None,
            scroll: 0,
            opt: 0,
            stash,
            hosts: vec![],
            services: vec![],
        };
        f.relayout();
        f
    }

    pub fn new_service(hosts: Vec<String>) -> Self {
        let stash = HashMap::from([("kind", "http".to_string()), ("mute", "no".into())]);
        let mut f = Self::new("Nuevo servicio".into(), Target::Service(None), stash);
        f.hosts = hosts;
        f.relayout();
        f
    }

    pub fn edit_service(svc: &ServiceCfg, hosts: Vec<String>) -> Self {
        let opt = |v: &Option<String>| v.clone().unwrap_or_default();
        let num = |v: Option<u64>| v.map_or(String::new(), |n| n.to_string());
        let stash = HashMap::from([
            ("name", svc.name.clone()),
            ("kind", svc.kind.clone()),
            ("target", opt(&svc.target)),
            ("unit", opt(&svc.unit)),
            ("command", opt(&svc.command)),
            ("host", svc.host.clone().unwrap_or_else(|| LOCAL.into())),
            ("suffix", opt(&svc.suffix)),
            ("link", opt(&svc.link)),
            ("expect_status", num(svc.expect_status.map(u64::from))),
            ("interval", svc.interval.clone()),
            ("timeout", svc.timeout.clone()),
            ("slow_ms", num(svc.slow_ms)),
            ("mute", if svc.mute { "sí" } else { "no" }.into()),
        ]);
        let mut f = Self::new(
            format!("Editar {}", svc.name),
            Target::Service(Some(svc.name.clone())),
            stash,
        );
        f.hosts = hosts;
        f.relayout();
        f
    }

    pub fn new_panel(page: usize, page_title: &str, services: Vec<String>) -> Self {
        let stash = HashMap::from([("kind", "status".to_string())]);
        let mut f = Self::new(format!("Panel en {page_title}"), Target::Panel(page), stash);
        f.services = services;
        f.relayout();
        f
    }

    pub fn new_page() -> Self {
        Self::new("Nueva página".into(), Target::Page, HashMap::new())
    }

    /// Reconstruye los campos según el tipo elegido, conservando los valores.
    fn relayout(&mut self) {
        use FieldKind::*;
        let kind = self.stash.get("kind").cloned().unwrap_or_default();
        let opts = |xs: &[&str]| Select(xs.iter().map(|s| s.to_string()).collect());
        let mut hosts = vec![LOCAL.to_string()];
        hosts.extend(self.hosts.iter().cloned());
        let mut specs: Vec<(&'static str, &'static str, &'static str, FieldKind)> = vec![];
        match &self.target {
            Target::Service(_) => {
                specs.push(("name", "Nombre", "único", Text));
                specs.push(("kind", "Tipo", "", opts(&SERVICE_KINDS)));
                match kind.as_str() {
                    "http" => {
                        specs.push(("target", "URL", "https://…", Text));
                        specs.push(("expect_status", "Código esperado", "vacío = < 400", Number));
                    }
                    "tcp" => specs.push(("target", "Destino", "host:puerto", Text)),
                    "ping" => specs.push(("target", "Destino", "host o IP", Text)),
                    "systemd" => {
                        specs.push(("unit", "Unidad", "p. ej. nginx.service", Text));
                        specs.push(("host", "Host", "", Select(hosts)));
                    }
                    "cmd" => {
                        specs.push(("command", "Comando", "stdout = número", Text));
                        specs.push(("host", "Host", "", Select(hosts)));
                        specs.push(("suffix", "Sufijo", "p. ej. %", Text));
                    }
                    _ => {}
                }
                specs.push(("link", "Link", "opcional", Text));
                specs.push(("interval", "Intervalo", "vacío = 30s", Text));
                specs.push(("timeout", "Timeout", "vacío = 10s", Text));
                specs.push(("slow_ms", "Lento desde (ms)", "opcional", Number));
                specs.push(("mute", "Silenciado", "", opts(&["no", "sí"])));
            }
            Target::Panel(_) => {
                specs.push(("kind", "Tipo", "", opts(&PANEL_KINDS)));
                specs.push(("title", "Título", "opcional", Text));
                specs.push((
                    "services",
                    "Servicios",
                    "ninguno = todos",
                    Multi(self.services.clone()),
                ));
                if kind == "chart" {
                    specs.push(("window", "Ventana", "vacío = 15m", opts(&WINDOWS)));
                }
            }
            Target::Page => specs.push(("title", "Título", "", Text)),
        }

        let focus_key = self.fields.get(self.focus).map(|f| f.key);
        self.fields = specs
            .into_iter()
            .map(|(key, label, help, kind)| {
                let mut value = self.stash.get(key).cloned().unwrap_or_default();
                if let Select(options) = &kind
                    && !options.contains(&value)
                {
                    value = options[0].clone();
                }
                Field {
                    key,
                    label,
                    help,
                    cursor: value.chars().count(),
                    kind,
                    value,
                }
            })
            .collect();
        self.focus = focus_key
            .and_then(|k| self.fields.iter().position(|f| f.key == k))
            .unwrap_or(0);
    }

    fn set(&mut self, i: usize, value: String) {
        let key = self.fields[i].key;
        self.stash.insert(key, value.clone());
        self.fields[i].value = value;
        if key == "kind" {
            self.relayout();
        }
    }

    pub fn value(&self, key: &str) -> String {
        self.fields
            .iter()
            .find(|f| f.key == key)
            .map(|f| f.value.clone())
            .unwrap_or_default()
    }

    /// Cambia la opción de un `Select` (`dir` = 1 siguiente, -1 anterior).
    pub fn cycle(&mut self, i: usize, dir: i32) {
        let Some(FieldKind::Select(options)) = self.fields.get(i).map(|f| f.kind.clone()) else {
            return;
        };
        self.focus = i;
        let cur = options
            .iter()
            .position(|o| *o == self.fields[i].value)
            .unwrap_or(0) as i32;
        let next = (cur + dir).rem_euclid(options.len() as i32) as usize;
        self.set(i, options[next].clone());
    }

    pub fn toggle(&mut self, i: usize, opt: usize) {
        let Some(FieldKind::Multi(options)) = self.fields.get(i).map(|f| f.kind.clone()) else {
            return;
        };
        let Some(name) = options.get(opt) else { return };
        self.focus = i;
        self.opt = opt;
        let mut sel: Vec<String> = self.fields[i]
            .selected()
            .into_iter()
            .map(String::from)
            .collect();
        match sel.iter().position(|s| s == name) {
            Some(p) => {
                sel.remove(p);
            }
            None => sel.push(name.clone()),
        }
        // Se guardan en el orden de las opciones, no en el de los toques.
        sel.sort_by_key(|s| options.iter().position(|o| o == s));
        self.set(i, sel.join("\n"));
    }

    pub fn focus_field(&mut self, i: usize) {
        if i < self.fields.len() {
            self.focus = i;
            self.opt = 0;
            self.fields[i].cursor = self.fields[i].value.chars().count();
        }
    }

    pub fn paste(&mut self, text: &str) {
        for c in text.chars().filter(|c| !c.is_control()) {
            self.insert(c);
        }
    }

    fn insert(&mut self, c: char) {
        let i = self.focus;
        let Some(f) = self.fields.get(i) else { return };
        match f.kind {
            FieldKind::Text => {}
            FieldKind::Number if c.is_ascii_digit() => {}
            _ => return,
        }
        let mut chars: Vec<char> = f.value.chars().collect();
        let at = f.cursor.min(chars.len());
        chars.insert(at, c);
        self.set(i, chars.into_iter().collect());
        self.fields[i].cursor = at + 1;
    }

    pub fn key(&mut self, k: KeyEvent) -> Outcome {
        let i = self.focus;
        let n = self.fields.len();
        let kind = self.fields.get(i).map(|f| f.kind.clone());
        match k.code {
            KeyCode::Esc => return Outcome::Cancel,
            KeyCode::Enter => return Outcome::Save,
            KeyCode::Tab | KeyCode::Down => self.focus_field((i + 1) % n.max(1)),
            KeyCode::BackTab | KeyCode::Up => self.focus_field((i + n.max(1) - 1) % n.max(1)),
            _ => match kind {
                Some(FieldKind::Select(_)) => match k.code {
                    KeyCode::Left => self.cycle(i, -1),
                    KeyCode::Right | KeyCode::Char(' ') => self.cycle(i, 1),
                    _ => {}
                },
                Some(FieldKind::Multi(options)) => match k.code {
                    KeyCode::Left => self.opt = self.opt.saturating_sub(1),
                    KeyCode::Right => {
                        self.opt = (self.opt + 1).min(options.len().saturating_sub(1))
                    }
                    KeyCode::Char(' ') => self.toggle(i, self.opt),
                    _ => {}
                },
                Some(_) => self.edit_text(k),
                None => {}
            },
        }
        Outcome::Continue
    }

    fn edit_text(&mut self, k: KeyEvent) {
        let i = self.focus;
        let len = self.fields[i].value.chars().count();
        let cur = self.fields[i].cursor.min(len);
        match k.code {
            KeyCode::Char(c) => self.insert(c),
            KeyCode::Backspace if cur > 0 => {
                let mut chars: Vec<char> = self.fields[i].value.chars().collect();
                chars.remove(cur - 1);
                self.set(i, chars.into_iter().collect());
                self.fields[i].cursor = cur - 1;
            }
            KeyCode::Delete if cur < len => {
                let mut chars: Vec<char> = self.fields[i].value.chars().collect();
                chars.remove(cur);
                self.set(i, chars.into_iter().collect());
                self.fields[i].cursor = cur;
            }
            KeyCode::Left => self.fields[i].cursor = cur.saturating_sub(1),
            KeyCode::Right => self.fields[i].cursor = (cur + 1).min(len),
            KeyCode::Home => self.fields[i].cursor = 0,
            KeyCode::End => self.fields[i].cursor = len,
            _ => {}
        }
    }

    /// Valores de servicio listos para `config_edit`; vacío = quitar la clave.
    pub fn service_values(&self) -> crate::config_edit::Values {
        crate::config_edit::SERVICE_KEYS
            .iter()
            .map(|&k| {
                let v = self.value(k);
                (
                    k,
                    if k == "host" && v == LOCAL {
                        String::new()
                    } else {
                        v
                    },
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    #[test]
    fn kind_change_relayouts_and_keeps_values() {
        let mut f = Form::new_service(vec!["vps".into()]);
        assert!(f.fields.iter().any(|x| x.key == "target"));
        f.focus_field(0);
        for c in "web".chars() {
            f.key(key(KeyCode::Char(c)));
        }
        // http → tcp → ping → systemd: aparecen unidad y host.
        f.cycle(1, 1);
        f.cycle(1, 1);
        f.cycle(1, 1);
        assert_eq!(f.value("kind"), "systemd");
        assert!(f.fields.iter().any(|x| x.key == "unit"));
        assert!(!f.fields.iter().any(|x| x.key == "target"));
        assert_eq!(f.value("name"), "web");
        let host = f.fields.iter().position(|x| x.key == "host").unwrap();
        f.cycle(host, 1);
        assert_eq!(f.value("host"), "vps");
        let vals = f.service_values();
        assert!(vals.contains(&("target", String::new())));
        assert!(vals.contains(&("host", "vps".into())));
    }

    #[test]
    fn text_editing_with_cursor() {
        let mut f = Form::new_page();
        for c in "Rd".chars() {
            f.key(key(KeyCode::Char(c)));
        }
        f.key(key(KeyCode::Left));
        f.key(key(KeyCode::Char('e')));
        assert_eq!(f.value("title"), "Red");
        f.key(key(KeyCode::End));
        f.key(key(KeyCode::Backspace));
        assert_eq!(f.value("title"), "Re");
        f.paste("des\n");
        assert_eq!(f.value("title"), "Redes");
        assert_eq!(f.key(key(KeyCode::Enter)), Outcome::Save);
    }

    #[test]
    fn numbers_only_accept_digits() {
        let mut f = Form::new_service(vec![]);
        let i = f.fields.iter().position(|x| x.key == "slow_ms").unwrap();
        f.focus_field(i);
        f.paste("12a3");
        assert_eq!(f.value("slow_ms"), "123");
    }

    #[test]
    fn panel_multi_select_keeps_option_order() {
        let mut f = Form::new_panel(0, "p", vec!["a".into(), "b".into(), "c".into()]);
        let i = f.fields.iter().position(|x| x.key == "services").unwrap();
        f.toggle(i, 2);
        f.toggle(i, 0);
        assert_eq!(f.fields[i].selected(), ["a", "c"]);
        f.toggle(i, 0);
        assert_eq!(f.fields[i].selected(), ["c"]);
        // La ventana solo aparece en gráficas.
        assert!(!f.fields.iter().any(|x| x.key == "window"));
        f.cycle(0, 1);
        assert!(f.fields.iter().any(|x| x.key == "window"));
    }
}
