use ratatui::{Frame, layout::Rect, style::Style, symbols, widgets::LineGauge};

use super::{Ctx, Panel};
use crate::input::{Action, HitMap};

/// Barras de uptime por servicio.
pub struct Gauge {
    pub title: String,
    pub ids: Vec<usize>,
}

impl Panel for Gauge {
    fn title(&self) -> &str {
        &self.title
    }

    fn render(&self, f: &mut Frame, area: Rect, ctx: &Ctx, hits: &mut HitMap) -> usize {
        let t = &ctx.app.theme;
        let ids = ctx.ids(&self.ids);
        let row_h = ctx.row_h();
        let visible = (area.height / row_h).max(1) as usize;
        let max = ids.len().saturating_sub(visible);
        let name_w = ids
            .iter()
            .map(|&i| ctx.app.services[i].name.chars().count())
            .max()
            .unwrap_or(0);

        for (row, &i) in ids
            .iter()
            .skip(ctx.scroll.min(max))
            .take(visible)
            .enumerate()
        {
            let svc = &ctx.app.services[i];
            let y = area.y + row as u16 * row_h;
            let r = Rect::new(area.x, y, area.width, 1);
            let up = svc.history.uptime();
            let color = match up {
                None => t.dim,
                Some(u) if u >= 0.99 => t.ok,
                Some(u) if u >= 0.9 => t.warn,
                Some(_) => t.err,
            };
            let pct = up.map_or("  …  ".into(), |u| format!("{:>5.1}%", u * 100.0));
            let gauge = LineGauge::default()
                .ratio(up.unwrap_or(0.0))
                .label(format!("{:<name_w$} {pct}", svc.name))
                .style(Style::new().fg(t.fg))
                .filled_style(Style::new().fg(color))
                .unfilled_style(Style::new().fg(t.border))
                .filled_symbol(symbols::line::THICK.horizontal)
                .unfilled_symbol(symbols::line::THICK.horizontal);
            f.render_widget(gauge, r);
            hits.push(
                Rect {
                    height: row_h.min(area.bottom() - y),
                    ..r
                },
                Action::Select(i),
            );
        }
        max
    }
}
