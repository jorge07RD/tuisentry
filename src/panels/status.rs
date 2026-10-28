use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
};

use super::{Ctx, Panel};
use crate::input::{Action, HitMap};

/// Tabla de servicios: estado, valor, uptime y link.
pub struct Status {
    pub title: String,
    pub ids: Vec<usize>,
}

impl Panel for Status {
    fn title(&self) -> &str {
        &self.title
    }

    fn render(&self, f: &mut Frame, area: Rect, ctx: &Ctx, hits: &mut HitMap) -> usize {
        let t = &ctx.app.theme;
        let ids = ctx.ids(&self.ids);
        let row_h = ctx.row_h();
        let visible = (area.height / row_h).max(1) as usize;
        let max = ids.len().saturating_sub(visible);
        let scroll = ctx.scroll.min(max);
        let show_uptime = area.width >= 36;

        if ids.is_empty() && ctx.app.view.down_only {
            f.render_widget(Line::styled("✓ nada caído", t.ok).centered(), area);
        }

        for (row, &i) in ids.iter().skip(scroll).take(visible).enumerate() {
            let svc = &ctx.app.services[i];
            let y = area.y + row as u16 * row_h;
            let line = Rect::new(area.x, y, area.width, 1);
            let touch = Rect::new(area.x, y, area.width, row_h.min(area.bottom() - y));
            let selected = ctx.app.selected == Some(i);
            if selected {
                f.buffer_mut().set_style(line, Style::new().bg(t.surface));
            }

            let up = svc.is_up();
            let (dot, color) = match up {
                _ if svc.paused => ("◌", t.dim),
                Some(true) => ("●", t.ok),
                Some(false) => ("●", t.err),
                None => ("○", t.dim),
            };
            let [c_dot, c_name, c_val, c_up, c_link] = Layout::horizontal([
                Constraint::Length(2),
                Constraint::Fill(1),
                Constraint::Length(10),
                Constraint::Length(if show_uptime { 8 } else { 0 }),
                Constraint::Length(3),
            ])
            .areas(line);

            f.render_widget(Span::styled(dot, color), c_dot);
            let name = Span::styled(svc.name.as_str(), t.fg);
            f.render_widget(
                if up == Some(false) {
                    name.fg(t.err).bold()
                } else {
                    name
                },
                c_name,
            );
            let val_color = if up == Some(false) { t.err } else { t.fg };
            f.render_widget(
                Line::styled(svc.value_label(), val_color).right_aligned(),
                c_val,
            );
            if show_uptime && let Some(u) = svc.history.uptime() {
                let c = match u {
                    u if u >= 0.99 => t.dim,
                    u if u >= 0.9 => t.warn,
                    _ => t.err,
                };
                f.render_widget(
                    Line::styled(format!("{:.1}%", u * 100.0), c).right_aligned(),
                    c_up,
                );
            }

            hits.push(touch, Action::Select(i));
            if svc.link.is_some() {
                f.render_widget(Line::styled("↗", t.accent).right_aligned(), c_link);
                hits.push(
                    Rect {
                        height: touch.height,
                        ..c_link
                    },
                    Action::OpenLink(i),
                );
            }
        }

        // Indicadores de que hay más filas.
        let right = area.right().saturating_sub(1);
        if scroll > 0 {
            f.render_widget(Span::styled("▲", t.dim), Rect::new(right, area.y, 1, 1));
        }
        if scroll < max {
            f.render_widget(
                Span::styled("▼", t.dim),
                Rect::new(right, area.bottom() - 1, 1, 1),
            );
        }
        max
    }
}
