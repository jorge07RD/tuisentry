use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, Instant},
};

use ratatui::crossterm::event::{
    Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use tokio::sync::mpsc;

use crate::{
    config::Config,
    engine::{Command, Event, Incident, ServiceInfo},
    input::{Action, HitMap, MENU_SCROLL},
    links,
    menu::{MenuState, Screen},
    panels::{self, Panel, fmt_span},
    prefs::UiPrefs,
    store::{History, MEMORY_WINDOW, unix_now},
    theme::Theme,
    view::View,
};

const FREE_MOUSE: Duration = Duration::from_secs(10);

pub struct Service {
    pub name: String,
    pub kind: String,
    pub target: Option<String>,
    pub host: Option<String>,
    pub link: Option<String>,
    pub suffix: String,
    pub paused: bool,
    pub muted: bool,
    pub history: History,
}

impl Service {
    #[cfg(test)]
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            kind: String::new(),
            target: None,
            host: None,
            link: None,
            suffix: "ms".into(),
            paused: false,
            muted: false,
            history: History::default(),
        }
    }

    fn from_info(info: ServiceInfo) -> Self {
        let mut history = History::default();
        for s in info.history {
            history.push(s);
        }
        Self {
            name: info.name,
            kind: info.kind,
            target: info.target,
            host: info.host,
            link: info.link,
            suffix: info.suffix,
            paused: info.paused,
            muted: info.muted,
            history,
        }
    }

    /// Último valor formateado (latencia, métrica o estado).
    pub fn value_label(&self) -> String {
        if self.paused {
            return "pausado".into();
        }
        match self.history.last() {
            None => "…".into(),
            Some(s) => match s.value {
                Some(v) if s.ok => fmt_value(v, &self.suffix),
                _ if s.ok => s.detail.clone(),
                _ => "caído".into(),
            },
        }
    }

    pub fn is_up(&self) -> Option<bool> {
        self.history.last().map(|s| s.ok)
    }
}

pub fn fmt_value(v: f64, suffix: &str) -> String {
    match suffix {
        "ms" if v >= 1000.0 => format!("{:.2} s", v / 1000.0),
        "ms" if v < 10.0 && v > 0.0 => format!("{v:.1} ms"),
        "ms" => format!("{v:.0} ms"),
        _ if v.fract() == 0.0 => format!("{v:.0}{suffix}"),
        _ => format!("{v:.1}{suffix}"),
    }
}

pub struct Page {
    pub title: String,
    pub panels: Vec<Box<dyn Panel>>,
    pub focus: usize,
    pub scroll: Vec<usize>,
}

/// Lo que solo puede hacer `main`, porque necesita la terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    EditConfig,
}

pub struct App {
    pub cfg: Config,
    pub cfg_path: PathBuf,
    pub services: Vec<Service>,
    pub pages: Vec<Page>,
    pub page: usize,
    pub zoom: Option<usize>,
    pub selected: Option<usize>,
    pub help: bool,
    pub menu: Option<MenuState>,
    pub view: View,
    pub theme: Theme,
    pub theme_name: String,
    pub toast: Option<(String, Instant)>,
    pub web: bool,
    pub quit: bool,
    /// Mientras sea futuro, la captura de mouse está apagada.
    pub mouse_free_until: Option<Instant>,
    pub hits: HitMap,
    pub effects: Vec<Effect>,
    /// Abiertos primero, luego los cerrados más recientes.
    pub incidents: Vec<Incident>,
    /// Uptime 24h/7d/30d por índice de servicio, calculado bajo demanda.
    pub stats: HashMap<usize, [Option<f64>; 3]>,
    /// Canales de alerta configurados (para el menú).
    pub channels: Vec<String>,
    cmd: mpsc::UnboundedSender<Command>,
    /// Dónde empezó el toque/clic actual y la última fila vista al arrastrar.
    press: Option<(u16, u16)>,
    drag_y: u16,
}

impl App {
    pub fn new(
        cfg: Config,
        cfg_path: PathBuf,
        web: bool,
        cmd: mpsc::UnboundedSender<Command>,
    ) -> Self {
        let prefs = UiPrefs::load();
        let theme_name = prefs
            .theme
            .or_else(|| cfg.theme.name.clone())
            .filter(|n| Theme::NAMES.contains(&n.as_str()))
            .unwrap_or_else(|| Theme::NAMES[0].into());
        let mut app = Self {
            cfg,
            cfg_path,
            services: vec![],
            pages: vec![],
            page: 0,
            zoom: None,
            selected: None,
            help: false,
            menu: None,
            view: prefs.view.unwrap_or_else(|| View::new(web)),
            theme: Theme::by_name(Some(&theme_name)),
            theme_name,
            toast: None,
            web,
            quit: false,
            mouse_free_until: None,
            hits: HitMap::default(),
            effects: vec![],
            incidents: vec![],
            stats: HashMap::new(),
            channels: vec![],
            cmd,
            press: None,
            drag_y: 0,
        };
        app.build_pages();
        app
    }

    /// Reconstruye las páginas a partir de la config y los servicios actuales,
    /// conservando la página, el foco y el scroll si siguen existiendo.
    fn build_pages(&mut self) {
        let old: Vec<(usize, Vec<usize>)> = self
            .pages
            .iter()
            .map(|p| (p.focus, p.scroll.clone()))
            .collect();
        self.pages = self
            .cfg
            .pages
            .iter()
            .enumerate()
            .map(|(pi, p)| {
                let panels: Vec<_> = p
                    .panels
                    .iter()
                    .filter_map(|pc| panels::build(pc, &self.services).ok())
                    .collect();
                let (focus, mut scroll) = old.get(pi).cloned().unwrap_or_default();
                scroll.resize(panels.len(), 0);
                Page {
                    title: p.title.clone(),
                    focus: focus.min(panels.len().saturating_sub(1)),
                    scroll,
                    panels,
                }
            })
            .collect();
        self.page = self.page.min(self.pages.len().saturating_sub(1));
        self.zoom = self
            .zoom
            .filter(|&z| z < self.pages[self.page].panels.len());
    }

    /// Segundos unix, la escala de `Sample::at`.
    pub fn now(&self) -> f64 {
        unix_now()
    }

    pub fn on_engine(&mut self, ev: Event) {
        match ev {
            Event::Snapshot(infos) => {
                self.services = infos.into_iter().map(Service::from_info).collect();
                self.selected = self.selected.filter(|&i| i < self.services.len());
                // Los índices pueden haber cambiado con la recarga.
                self.stats.clear();
                self.build_pages();
            }
            Event::Sample(i, s) => {
                if let Some(svc) = self.services.get_mut(i) {
                    svc.history.push(s);
                }
            }
            Event::Flags { idx, paused, muted } => {
                if let Some(svc) = self.services.get_mut(idx) {
                    svc.paused = paused;
                    svc.muted = muted;
                }
            }
            Event::Incidents(list) => self.incidents = list,
            Event::Channels(list) => self.channels = list,
            Event::Stats { idx, uptime } => {
                self.stats.insert(idx, uptime);
            }
            Event::Toast(msg) => self.toast(msg),
        }
    }

    pub fn page(&self) -> &Page {
        &self.pages[self.page]
    }

    pub fn mouse_wanted(&self) -> bool {
        self.mouse_free_until.is_none_or(|t| Instant::now() >= t)
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    fn send(&mut self, c: Command) {
        if self.cmd.send(c).is_err() {
            self.toast("el motor no responde");
        }
    }

    fn save_prefs(&self) {
        UiPrefs {
            theme: Some(self.theme_name.clone()),
            view: Some(self.view.clone()),
        }
        .save();
    }

    /// Valida la config en disco; si está bien, la aplica y pide al motor que recargue.
    pub fn reload(&mut self) {
        match Config::load(&self.cfg_path) {
            Ok(cfg) => {
                self.cfg = cfg;
                self.send(Command::Reload);
            }
            Err(e) => self.toast(format!("config inválida: {e:#}")),
        }
    }

    pub fn apply(&mut self, a: Action) {
        let n_panels = self.page().panels.len();
        match a {
            Action::Page(i) if i < self.pages.len() => {
                self.page = i;
                self.zoom = None;
            }
            Action::Page(_) | Action::Block => {}
            Action::PrevPanel | Action::NextPanel if n_panels > 0 => {
                let page = &mut self.pages[self.page];
                page.focus = if a == Action::NextPanel {
                    (page.focus + 1) % n_panels
                } else {
                    (page.focus + n_panels - 1) % n_panels
                };
                if self.zoom.is_some() {
                    self.zoom = Some(page.focus);
                }
            }
            Action::PrevPanel | Action::NextPanel => {}
            Action::Zoom(i) => {
                self.pages[self.page].focus = i;
                self.zoom = if self.zoom == Some(i) { None } else { Some(i) };
            }
            Action::Select(i) => {
                self.menu = None;
                self.selected = Some(i);
            }
            Action::OpenLink(i) => match self.services[i].link.clone() {
                Some(url) => {
                    let msg = links::open(&url, self.web);
                    self.toast(msg);
                }
                None => self.toast(format!("{} no tiene link", self.services[i].name)),
            },
            Action::CopyUrl(i) => {
                if let Some(url) = self.services[i].link.clone() {
                    let msg = links::copy(&url, self.web);
                    self.toast(msg);
                }
            }
            Action::Check(target) => {
                self.send(Command::CheckNow(target));
                match target {
                    Some(i) => self.toast(format!("⟳ comprobando {}", self.services[i].name)),
                    None => self.toast("⟳ comprobando todo"),
                }
            }
            Action::TogglePause(i) => {
                let on = !self.services[i].paused;
                self.send(Command::SetPaused(Some(i), on));
            }
            Action::ToggleMute(i) => {
                let on = !self.services[i].muted;
                self.send(Command::SetMuted(i, on));
            }
            Action::TestAlert => {
                self.send(Command::TestAlert);
                if !self.channels.is_empty() {
                    self.toast("enviando alerta de prueba…");
                }
            }
            Action::PauseAll => {
                let on = !self.services.iter().all(|s| s.paused);
                self.send(Command::SetPaused(None, on));
                self.toast(if on {
                    "‖ todo en pausa"
                } else {
                    "▶ todo reanudado"
                });
            }
            Action::Help => {
                self.menu = None;
                self.help = !self.help;
            }
            Action::Close => {
                if self.menu.is_some() {
                    self.menu = None;
                } else if self.help {
                    self.help = false;
                } else if self.selected.is_some() {
                    self.selected = None;
                } else {
                    self.zoom = None;
                }
            }
            Action::FreeMouse => {
                self.menu = None;
                self.mouse_free_until = Some(Instant::now() + FREE_MOUSE);
                self.toast("Mouse libre 10 s: toca o selecciona el link");
            }
            Action::Quit => self.quit = true,
            Action::Menu => {
                self.menu = match self.menu {
                    Some(_) => None,
                    None => Some(MenuState::default()),
                };
            }
            Action::MenuBack => {
                if let Some(m) = &mut self.menu {
                    if !m.query.is_empty() {
                        m.query.clear();
                    } else if m.stack.pop().is_none() {
                        self.menu = None;
                    }
                    if let Some(m) = &mut self.menu {
                        m.scroll = 0;
                        m.sel = 0;
                    }
                }
            }
            Action::MenuGo(s) => {
                if let Screen::Service(i) = s {
                    self.send(Command::Stats(i));
                }
                // Desde fuera del menú (p. ej. el detalle) se abre directo en esa pantalla.
                self.selected = None;
                match &mut self.menu {
                    Some(m) => {
                        m.stack.push(s);
                        m.query.clear();
                        m.scroll = 0;
                        m.sel = 0;
                    }
                    None => self.menu = Some(MenuState::at(s)),
                }
            }
            Action::SetWindow(secs) => {
                self.view.window = (secs > 0).then_some(f64::from(secs));
                self.save_prefs();
                let label = self.view.window.map_or("la de cada panel".into(), fmt_span);
                self.toast(format!("ventana: {label}"));
            }
            Action::SetSort(s) => {
                self.view.sort = s;
                self.save_prefs();
            }
            Action::ToggleDownOnly => {
                self.view.down_only = !self.view.down_only;
                self.save_prefs();
            }
            Action::ToggleTouch => {
                self.view.touch = !self.view.touch;
                self.save_prefs();
            }
            Action::SetTheme(i) => {
                if let Some(&name) = Theme::NAMES.get(usize::from(i)) {
                    self.theme_name = name.into();
                    self.theme = Theme::by_name(Some(name));
                    self.save_prefs();
                }
            }
            Action::Reload => self.reload(),
            Action::Export(i) => {
                let secs = self.view.window.unwrap_or(MEMORY_WINDOW);
                self.send(Command::Export(i, secs));
                self.toast(format!("exportando {}…", self.services[i].name));
            }
            Action::EditConfig => {
                self.menu = None;
                self.effects.push(Effect::EditConfig);
            }
        }
    }

    pub fn on_event(&mut self, ev: TermEvent) {
        match ev {
            TermEvent::Key(k) if k.kind == KeyEventKind::Press => self.on_key(k),
            TermEvent::Mouse(m) => self.on_mouse(m),
            _ => {}
        }
    }

    fn on_key(&mut self, k: KeyEvent) {
        // Cualquier tecla devuelve el mouse a la app.
        self.mouse_free_until = None;
        if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
            return self.apply(Action::Quit);
        }
        if self.menu.is_some() {
            return self.menu_key(k);
        }
        let a = match k.code {
            KeyCode::Char('q') => Action::Quit,
            KeyCode::Esc => Action::Close,
            KeyCode::Char('?') => Action::Help,
            KeyCode::Char(' ' | ':') => Action::Menu,
            KeyCode::Tab => Action::Page((self.page + 1) % self.pages.len()),
            KeyCode::BackTab => Action::Page((self.page + self.pages.len() - 1) % self.pages.len()),
            KeyCode::Char(c @ '1'..='9') => Action::Page(c as usize - '1' as usize),
            KeyCode::Left | KeyCode::Char('h') => Action::PrevPanel,
            KeyCode::Right | KeyCode::Char('l') => Action::NextPanel,
            KeyCode::Enter | KeyCode::Char('z') => Action::Zoom(self.page().focus),
            KeyCode::Char('r') => Action::Check(self.selected),
            KeyCode::Char('o') => match self.selected {
                Some(i) => Action::OpenLink(i),
                None => return,
            },
            KeyCode::Char('m') => Action::FreeMouse,
            KeyCode::Down | KeyCode::Char('j') => return self.scroll_focused(1),
            KeyCode::Up | KeyCode::Char('k') => return self.scroll_focused(-1),
            _ => return,
        };
        self.apply(a);
    }

    /// Con el menú abierto el teclado escribe en la búsqueda.
    fn menu_key(&mut self, k: KeyEvent) {
        let items = match &self.menu {
            Some(m) => m.items(self),
            None => return,
        };
        let visible = self.hits.menu_rows.max(1);
        let Some(m) = &mut self.menu else { return };
        match k.code {
            KeyCode::Esc => return self.apply(Action::MenuBack),
            KeyCode::Backspace if m.query.is_empty() => return self.apply(Action::MenuBack),
            KeyCode::Backspace => {
                m.query.pop();
                m.sel = 0;
                m.scroll = 0;
            }
            KeyCode::Down => m.sel = (m.sel + 1).min(items.len().saturating_sub(1)),
            KeyCode::Up => m.sel = m.sel.saturating_sub(1),
            KeyCode::Enter => {
                if let Some(item) = items.get(m.sel) {
                    let a = item.action;
                    return self.apply(a);
                }
            }
            // Espacio con la búsqueda vacía cierra, igual que lo abrió.
            KeyCode::Char(' ') if m.query.is_empty() => {
                self.menu = None;
                return;
            }
            KeyCode::Char(c) => {
                m.query.push(c);
                m.sel = 0;
                m.scroll = 0;
            }
            _ => {}
        }
        // Mantiene la selección a la vista.
        if m.sel < m.scroll {
            m.scroll = m.sel;
        } else if m.sel >= m.scroll + visible {
            m.scroll = m.sel + 1 - visible;
        }
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.press = Some((m.column, m.row));
                self.drag_y = m.row;
            }
            // Un toque cuenta como clic solo si el dedo casi no se movió;
            // así deslizar para hacer scroll no dispara acciones.
            MouseEventKind::Up(MouseButton::Left) => {
                if let Some((x, y)) = self.press.take()
                    && m.row.abs_diff(y) <= 1
                    && m.column.abs_diff(x) <= 2
                    && let Some(a) = self.hits.at(x, y)
                {
                    self.apply(a);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some((x, y)) = self.press {
                    let delta = self.drag_y as i32 - m.row as i32;
                    self.drag_y = m.row;
                    self.scroll_at(x, y, delta);
                }
            }
            MouseEventKind::ScrollDown => self.scroll_at(m.column, m.row, 1),
            MouseEventKind::ScrollUp => self.scroll_at(m.column, m.row, -1),
            _ => {}
        }
    }

    fn scroll_at(&mut self, x: u16, y: u16, delta: i32) {
        let Some((target, max)) = self.hits.scroll_at(x, y) else {
            return;
        };
        let s = if target == MENU_SCROLL {
            match &mut self.menu {
                Some(m) => &mut m.scroll,
                None => return,
            }
        } else {
            &mut self.pages[self.page].scroll[target]
        };
        *s = (*s as i32 + delta).clamp(0, max as i32) as usize;
    }

    fn scroll_focused(&mut self, delta: i32) {
        let page = &mut self.pages[self.page];
        let max = self.hits.max_scroll(page.focus);
        if let Some(s) = page.scroll.get_mut(page.focus) {
            *s = (*s as i32 + delta).clamp(0, max as i32) as usize;
        }
    }

    /// Abre el menú en una pantalla concreta (para tests y atajos).
    #[cfg(test)]
    pub fn open_menu(&mut self, s: Screen) {
        self.menu = Some(MenuState::at(s));
    }
}

/// Construye una `App` sin motor, con servicios ya cargados (tests).
#[cfg(test)]
pub fn test_app(services: Vec<Service>) -> App {
    let cfg: Config = toml::from_str(
        r#"
        [[service]]
        name = "x"
        kind = "tcp"
        target = "localhost:1"
        [[page]]
        title = "p"
        [[page.panel]]
        kind = "status"
        "#,
    )
    .unwrap();
    let (cmd, _rx) = mpsc::unbounded_channel();
    let mut app = App::new(cfg, PathBuf::new(), false, cmd);
    app.services = services;
    app
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values() {
        assert_eq!(fmt_value(42.4, "ms"), "42 ms");
        assert_eq!(fmt_value(1530.0, "ms"), "1.53 s");
        assert_eq!(fmt_value(0.5, "ms"), "0.5 ms");
        assert_eq!(fmt_value(73.0, "%"), "73%");
        assert_eq!(fmt_value(0.75, ""), "0.8");
    }

    #[test]
    fn menu_navigation_and_search() {
        let mut app = test_app(vec![
            Service::new("chelitonegro"),
            Service::new("portafolio"),
        ]);
        app.apply(Action::Menu);
        app.apply(Action::MenuGo(Screen::View));
        assert_eq!(app.menu.as_ref().unwrap().screen(), Screen::View);
        app.apply(Action::MenuBack);
        assert_eq!(app.menu.as_ref().unwrap().screen(), Screen::Root);
        app.apply(Action::MenuBack);
        assert!(app.menu.is_none());

        app.open_menu(Screen::Root);
        app.menu.as_mut().unwrap().query = "porta".into();
        let found = app.menu.as_ref().unwrap().items(&app);
        assert!(
            found
                .iter()
                .any(|i| i.action == Action::MenuGo(Screen::Service(1)))
        );
        app.menu.as_mut().unwrap().query = "1h".into();
        let found = app.menu.as_ref().unwrap().items(&app);
        assert!(found.iter().any(|i| i.action == Action::SetWindow(3600)));
    }
}
