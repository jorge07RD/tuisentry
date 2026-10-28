use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use tokio::sync::{Notify, mpsc};

use crate::{
    checks,
    config::Config,
    input::{Action, HitMap},
    links,
    panels::{self, Panel},
    store::{History, Sample},
    theme::Theme,
};

/// Muestras guardadas por servicio (~5 h a 30 s, ~10 min a 1 s).
const HISTORY: usize = 600;
const FREE_MOUSE: Duration = Duration::from_secs(10);

pub struct Service {
    pub name: String,
    pub kind: String,
    pub target: Option<String>,
    pub host: Option<String>,
    pub link: Option<String>,
    pub suffix: String,
    pub history: History,
    notify: Arc<Notify>,
}

impl Service {
    /// Último valor formateado (latencia, métrica o estado).
    pub fn value_label(&self) -> String {
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

pub struct App {
    pub services: Vec<Service>,
    pub pages: Vec<Page>,
    pub page: usize,
    pub zoom: Option<usize>,
    pub selected: Option<usize>,
    pub help: bool,
    pub toast: Option<(String, Instant)>,
    pub web: bool,
    pub theme: Theme,
    pub quit: bool,
    /// Mientras sea futuro, la captura de mouse está apagada.
    pub mouse_free_until: Option<Instant>,
    pub hits: HitMap,
    start: Instant,
    /// Dónde empezó el toque/clic actual y la última fila vista al arrastrar.
    press: Option<(u16, u16)>,
    drag_y: u16,
}

impl App {
    pub fn new(
        cfg: &Config,
        web: bool,
        start: tokio::time::Instant,
        tx: mpsc::UnboundedSender<(usize, Sample)>,
    ) -> Result<Self> {
        let mut services = Vec::with_capacity(cfg.services.len());
        for (i, sc) in cfg.services.iter().enumerate() {
            let check: Arc<dyn checks::Check> = checks::build(sc, cfg)?.into();
            let notify = Arc::new(Notify::new());
            services.push(Service {
                name: sc.name.clone(),
                kind: sc.kind.clone(),
                target: sc
                    .target
                    .clone()
                    .or_else(|| sc.unit.clone())
                    .or_else(|| sc.command.clone()),
                host: sc.host.clone(),
                link: sc.link.clone().or_else(|| check.default_link()),
                suffix: sc
                    .suffix
                    .clone()
                    .unwrap_or_else(|| check.default_suffix().into()),
                history: History::new(HISTORY),
                notify: notify.clone(),
            });
            checks::spawn(i, check, sc, notify, start, tx.clone());
        }
        let mut pages = Vec::with_capacity(cfg.pages.len());
        for p in &cfg.pages {
            let panels = p
                .panels
                .iter()
                .map(|pc| panels::build(pc, &services, web))
                .collect::<Result<Vec<_>>>()?;
            pages.push(Page {
                title: p.title.clone(),
                scroll: vec![0; panels.len()],
                panels,
                focus: 0,
            });
        }
        Ok(Self {
            services,
            pages,
            page: 0,
            zoom: None,
            selected: None,
            help: false,
            toast: None,
            web,
            theme: Theme::by_name(cfg.theme.name.as_deref()),
            quit: false,
            mouse_free_until: None,
            hits: HitMap::default(),
            start: start.into_std(),
            press: None,
            drag_y: 0,
        })
    }

    /// Segundos desde el arranque, en la misma escala que `Sample::at`.
    pub fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    pub fn record(&mut self, idx: usize, s: Sample) {
        if let Some(svc) = self.services.get_mut(idx) {
            svc.history.push(s);
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
            Action::Select(i) => self.selected = Some(i),
            Action::OpenLink(i) => match self.services[i].link.clone() {
                Some(url) => {
                    let msg = links::open(&url, self.web);
                    self.toast(msg);
                }
                None => self.toast(format!("{} no tiene link", self.services[i].name)),
            },
            Action::Check(Some(i)) => {
                self.services[i].notify.notify_one();
                self.toast(format!("⟳ comprobando {}", self.services[i].name));
            }
            Action::Check(None) => {
                for s in &self.services {
                    s.notify.notify_one();
                }
                self.toast("⟳ comprobando todo");
            }
            Action::Help => self.help = !self.help,
            Action::Close => {
                if self.help {
                    self.help = false;
                } else if self.selected.is_some() {
                    self.selected = None;
                } else {
                    self.zoom = None;
                }
            }
            Action::FreeMouse => {
                self.mouse_free_until = Some(Instant::now() + FREE_MOUSE);
                self.toast("Mouse libre 10 s: toca o selecciona el link");
            }
            Action::Quit => self.quit = true,
        }
    }

    pub fn on_event(&mut self, ev: Event) {
        match ev {
            Event::Key(k) if k.kind == KeyEventKind::Press => self.on_key(k),
            Event::Mouse(m) => self.on_mouse(m),
            _ => {}
        }
    }

    fn on_key(&mut self, k: KeyEvent) {
        // Cualquier tecla devuelve el mouse a la app.
        self.mouse_free_until = None;
        let a = match k.code {
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
            KeyCode::Char('q') => Action::Quit,
            KeyCode::Esc => Action::Close,
            KeyCode::Char('?') => Action::Help,
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
        if let Some((panel, max)) = self.hits.scroll_at(x, y) {
            let s = &mut self.pages[self.page].scroll[panel];
            *s = (*s as i32 + delta).clamp(0, max as i32) as usize;
        }
    }

    fn scroll_focused(&mut self, delta: i32) {
        let page = &mut self.pages[self.page];
        let max = self.hits.max_scroll(page.focus);
        if let Some(s) = page.scroll.get_mut(page.focus) {
            *s = (*s as i32 + delta).clamp(0, max as i32) as usize;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fmt_value;

    #[test]
    fn values() {
        assert_eq!(fmt_value(42.4, "ms"), "42 ms");
        assert_eq!(fmt_value(1530.0, "ms"), "1.53 s");
        assert_eq!(fmt_value(0.5, "ms"), "0.5 ms");
        assert_eq!(fmt_value(73.0, "%"), "73%");
        assert_eq!(fmt_value(0.75, ""), "0.8");
    }
}
