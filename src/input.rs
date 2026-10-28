use ratatui::{
    layout::{Position, Rect},
    style::Color,
};

use crate::{menu::Screen, view::Sort};

/// Identificador de scroll del menú en el `HitMap` (los paneles usan su índice).
pub const MENU_SCROLL: usize = usize::MAX;

/// Todo lo que se puede hacer con un toque, un clic o una tecla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Page(usize),
    PrevPanel,
    NextPanel,
    Zoom(usize),
    Select(usize),
    OpenLink(usize),
    CopyUrl(usize),
    /// Exporta a CSV la ventana actual del servicio.
    Export(usize),
    /// `None` = todos los servicios.
    Check(Option<usize>),
    TogglePause(usize),
    /// Pausa todo, o reanuda todo si ya estaba todo pausado.
    PauseAll,
    Help,
    Close,
    /// Suelta el mouse unos segundos para tocar o seleccionar links en la terminal web.
    FreeMouse,
    Quit,
    /// Área que absorbe clics sin hacer nada (cuerpo de un popup).
    Block,
    /// Abre o cierra el menú.
    Menu,
    MenuBack,
    MenuGo(Screen),
    /// Segundos; 0 = la ventana propia de cada panel.
    SetWindow(u32),
    SetSort(Sort),
    ToggleDownOnly,
    ToggleTouch,
    SetTheme(u8),
    Reload,
    EditConfig,
}

/// Texto que, tras dibujar, se reescribe como hipervínculo OSC 8.
#[derive(Debug, Clone)]
pub struct LinkSpan {
    pub pos: Position,
    pub text: String,
    pub url: String,
    pub fg: Color,
}

/// Se reconstruye en cada frame: cada widget interactivo registra su área.
#[derive(Debug, Default)]
pub struct HitMap {
    hits: Vec<(Rect, Action)>,
    /// (área, índice de panel, scroll máximo)
    scrolls: Vec<(Rect, usize, usize)>,
    /// Filas de entradas que caben en el menú (para mantener la selección a la vista).
    pub menu_rows: usize,
    pub links: Vec<LinkSpan>,
}

impl HitMap {
    pub fn push(&mut self, area: Rect, action: Action) {
        if !area.is_empty() {
            self.hits.push((area, action));
        }
    }

    pub fn scroll(&mut self, area: Rect, panel: usize, max: usize) {
        if max > 0 {
            self.scrolls.push((area, panel, max));
        }
    }

    /// Popup modal: lo de debajo deja de ser interactivo y tocar fuera cierra.
    pub fn modal(&mut self, screen: Rect, popup: Rect) {
        self.scrolls.clear();
        self.links.clear();
        self.push(screen, Action::Close);
        self.push(popup, Action::Block);
    }

    pub fn link(&mut self, pos: Position, text: &str, url: &str, fg: Color) {
        self.links.push(LinkSpan {
            pos,
            text: text.into(),
            url: url.into(),
            fg,
        });
    }

    /// Lo registrado al final queda encima.
    pub fn at(&self, x: u16, y: u16) -> Option<Action> {
        let p = Position { x, y };
        self.hits
            .iter()
            .rev()
            .find(|(r, _)| r.contains(p))
            .map(|(_, a)| *a)
    }

    pub fn scroll_at(&self, x: u16, y: u16) -> Option<(usize, usize)> {
        let p = Position { x, y };
        self.scrolls
            .iter()
            .rev()
            .find(|(r, ..)| r.contains(p))
            .map(|&(_, i, m)| (i, m))
    }

    pub fn max_scroll(&self, panel: usize) -> usize {
        self.scrolls
            .iter()
            .find(|(_, i, _)| *i == panel)
            .map_or(0, |&(.., m)| m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topmost_hit_wins() {
        let mut h = HitMap::default();
        h.push(Rect::new(0, 0, 10, 10), Action::Select(0));
        h.push(Rect::new(2, 2, 3, 1), Action::OpenLink(0));
        assert_eq!(h.at(3, 2), Some(Action::OpenLink(0)));
        assert_eq!(h.at(0, 0), Some(Action::Select(0)));
        assert_eq!(h.at(20, 20), None);
    }

    #[test]
    fn modal_covers_everything_below() {
        let mut h = HitMap::default();
        h.push(Rect::new(0, 0, 10, 1), Action::Page(1));
        h.scroll(Rect::new(0, 1, 10, 9), 0, 5);
        h.modal(Rect::new(0, 0, 80, 24), Rect::new(20, 5, 40, 10));
        h.push(Rect::new(22, 12, 8, 1), Action::Check(Some(0)));
        assert_eq!(h.at(1, 0), Some(Action::Close));
        assert_eq!(h.at(30, 6), Some(Action::Block));
        assert_eq!(h.at(23, 12), Some(Action::Check(Some(0))));
        assert_eq!(h.scroll_at(1, 3), None);
    }
}
