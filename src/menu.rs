//! Menú táctil: pantallas con entradas generadas a partir del estado de la app,
//! y búsqueda que recorre todas a la vez (paleta de comandos).

use crate::{
    app::App, input::Action, panels::fmt_span, store::unix_now, text::fold, theme::Theme,
    time::human, view::Sort,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Root,
    Services,
    Service(usize),
    History,
    View,
    Window,
    Sort,
    Settings,
    Theme,
}

/// Ventanas de tiempo que ofrece el menú, en segundos.
pub const WINDOWS: [u32; 5] = [300, 900, 3600, 6 * 3600, 24 * 3600];

#[derive(Debug, Clone)]
pub struct MenuItem {
    pub icon: &'static str,
    pub label: String,
    /// Texto atenuado a la derecha (valor actual, contador…).
    pub badge: Option<String>,
    /// `Some` = interruptor, con su estado.
    pub on: Option<bool>,
    pub action: Action,
}

impl MenuItem {
    fn new(icon: &'static str, label: impl Into<String>, action: Action) -> Self {
        Self {
            icon,
            label: label.into(),
            badge: None,
            on: None,
            action,
        }
    }

    fn badge(mut self, b: impl Into<String>) -> Self {
        self.badge = Some(b.into());
        self
    }

    fn toggle(mut self, on: bool) -> Self {
        self.on = Some(on);
        self
    }

    pub fn is_submenu(&self) -> bool {
        matches!(self.action, Action::MenuGo(_))
    }
}

#[derive(Debug, Default)]
pub struct MenuState {
    pub stack: Vec<Screen>,
    pub query: String,
    pub scroll: usize,
    /// Entrada resaltada con el teclado.
    pub sel: usize,
}

impl MenuState {
    pub fn at(screen: Screen) -> Self {
        Self {
            stack: vec![screen],
            ..Self::default()
        }
    }

    pub fn screen(&self) -> Screen {
        self.stack.last().copied().unwrap_or(Screen::Root)
    }

    /// Entradas visibles: las de la pantalla actual o los resultados de búsqueda.
    pub fn items(&self, app: &App) -> Vec<MenuItem> {
        if self.query.is_empty() {
            items(self.screen(), app)
        } else {
            search(&self.query, app)
        }
    }
}

pub fn title(screen: Screen, app: &App) -> String {
    match screen {
        Screen::Root => "Menú".into(),
        Screen::Services => "Servicios".into(),
        Screen::Service(i) => app.services.get(i).map_or("?".into(), |s| s.name.clone()),
        Screen::History => "Historial".into(),
        Screen::View => "Vista".into(),
        Screen::Window => "Ventana de tiempo".into(),
        Screen::Sort => "Ordenar".into(),
        Screen::Settings => "Ajustes".into(),
        Screen::Theme => "Tema".into(),
    }
}

pub fn items(screen: Screen, app: &App) -> Vec<MenuItem> {
    use Action as A;
    let check = |b: bool| if b { "✓" } else { " " };
    match screen {
        Screen::Root => {
            let down = app
                .services
                .iter()
                .filter(|s| s.is_up() == Some(false))
                .count();
            let mut services = MenuItem::new("◉", "Servicios", A::MenuGo(Screen::Services));
            if down > 0 {
                services = services.badge(format!("{down} caídos"));
            }
            let open = app.incidents.iter().filter(|i| i.end.is_none()).count();
            let mut history = MenuItem::new("◷", "Historial", A::MenuGo(Screen::History));
            if open > 0 {
                history = history.badge(format!("{open} abiertos"));
            }
            vec![
                services,
                history,
                MenuItem::new("▦", "Vista", A::MenuGo(Screen::View)),
                MenuItem::new("⚙", "Ajustes", A::MenuGo(Screen::Settings)),
                MenuItem::new("?", "Ayuda", A::Help),
                MenuItem::new("✕", "Salir", A::Quit),
            ]
        }
        Screen::Services => {
            let all_paused = !app.services.is_empty() && app.services.iter().all(|s| s.paused);
            let mut v = vec![
                MenuItem::new("⟳", "Comprobar todo", A::Check(None)),
                if all_paused {
                    MenuItem::new("▶", "Reanudar todo", A::PauseAll)
                } else {
                    MenuItem::new("‖", "Pausar todo", A::PauseAll)
                },
            ];
            v.extend(app.services.iter().enumerate().map(|(i, s)| {
                let icon = match (s.paused, s.is_up()) {
                    (true, _) => "◌",
                    (_, Some(false)) => "✗",
                    (_, Some(true)) => "●",
                    _ => "○",
                };
                MenuItem::new(icon, s.name.clone(), A::MenuGo(Screen::Service(i)))
                    .badge(s.value_label())
            }));
            v
        }
        Screen::Service(i) => {
            let Some(s) = app.services.get(i) else {
                return vec![];
            };
            let mut v = vec![MenuItem::new("ⓘ", "Ver detalle", A::Select(i))];
            if s.link.is_some() {
                v.push(MenuItem::new("↗", "Abrir recurso", A::OpenLink(i)));
                v.push(MenuItem::new("⧉", "Copiar URL", A::CopyUrl(i)));
            }
            v.push(MenuItem::new("⟳", "Comprobar ahora", A::Check(Some(i))));
            v.push(MenuItem::new("‖", "Pausado", A::TogglePause(i)).toggle(s.paused));
            let pct = |u: Option<f64>| match u {
                None => "–".into(),
                Some(u) if u >= 0.9995 => "100%".into(),
                Some(u) => format!("{:.1}%", u * 100.0),
            };
            let uptime = app
                .stats
                .get(&i)
                .map_or("calculando…".into(), |u| u.map(pct).join(" · "));
            v.push(MenuItem::new("◔", "Uptime 24h·7d·30d", A::Block).badge(uptime));
            v.push(MenuItem::new("⇩", "Exportar CSV", A::Export(i)));
            v
        }
        Screen::History => {
            if app.incidents.is_empty() {
                return vec![MenuItem::new("✓", "Sin incidentes registrados", A::Block)];
            }
            let now = unix_now();
            app.incidents
                .iter()
                .map(|inc| {
                    let target = app
                        .services
                        .iter()
                        .position(|s| s.name == inc.service)
                        .map_or(A::Block, |i| A::MenuGo(Screen::Service(i)));
                    match inc.end {
                        None => MenuItem::new("✗", format!("{} · caído", inc.service), target)
                            .badge(format!("hace {}", human(now - inc.start))),
                        Some(end) => MenuItem::new("✓", inc.service.clone(), target).badge(
                            format!("{} · hace {}", human(end - inc.start), human(now - end)),
                        ),
                    }
                })
                .collect()
        }
        Screen::View => {
            let window = app.view.window.map_or("por panel".into(), fmt_span);
            vec![
                MenuItem::new("◷", "Ventana de tiempo", A::MenuGo(Screen::Window)).badge(window),
                MenuItem::new("⇅", "Ordenar", A::MenuGo(Screen::Sort)).badge(app.view.sort.label()),
                MenuItem::new("✗", "Solo caídos", A::ToggleDownOnly).toggle(app.view.down_only),
                MenuItem::new("≡", "Filas amplias (táctil)", A::ToggleTouch).toggle(app.view.touch),
            ]
        }
        Screen::Window => {
            let mut v = vec![MenuItem::new(
                check(app.view.window.is_none()),
                "La de cada panel",
                A::SetWindow(0),
            )];
            v.extend(WINDOWS.iter().map(|&w| {
                let current = app.view.window == Some(f64::from(w));
                MenuItem::new(check(current), fmt_span(f64::from(w)), A::SetWindow(w))
            }));
            v
        }
        Screen::Sort => Sort::ALL
            .iter()
            .map(|&s| {
                MenuItem::new(
                    check(app.view.sort == s),
                    format!("Por {}", s.label()),
                    A::SetSort(s),
                )
            })
            .collect(),
        Screen::Settings => {
            let mut v = vec![
                MenuItem::new("◐", "Tema", A::MenuGo(Screen::Theme)).badge(app.theme_name.clone()),
                MenuItem::new("⟳", "Recargar config", A::Reload),
                MenuItem::new("✎", "Editar config en $EDITOR", A::EditConfig),
            ];
            if app.web {
                v.push(MenuItem::new("⇱", "Mouse libre 10 s", A::FreeMouse));
            }
            v
        }
        Screen::Theme => Theme::NAMES
            .iter()
            .enumerate()
            .map(|(i, &name)| {
                MenuItem::new(check(app.theme_name == name), name, A::SetTheme(i as u8))
            })
            .collect(),
    }
}

/// Pantallas que la búsqueda recorre, además de una por servicio.
const SEARCHABLE: [Screen; 7] = [
    Screen::Root,
    Screen::Services,
    Screen::View,
    Screen::Window,
    Screen::Sort,
    Screen::Settings,
    Screen::Theme,
];

/// Busca en todas las pantallas. Cada resultado lleva su contexto en la etiqueta.
pub fn search(query: &str, app: &App) -> Vec<MenuItem> {
    let q = fold(query);
    let screens = SEARCHABLE
        .into_iter()
        .chain((0..app.services.len()).map(Screen::Service));
    let mut out = vec![];
    for screen in screens {
        let ctx = title(screen, app);
        for mut item in items(screen, app) {
            let is_service = matches!(item.action, Action::MenuGo(Screen::Service(_)));
            let label = if screen == Screen::Root || is_service {
                item.label.clone()
            } else {
                format!("{ctx} · {}", item.label)
            };
            if fold(&label).contains(&q) {
                item.label = label;
                out.push(item);
            }
        }
    }
    out
}
