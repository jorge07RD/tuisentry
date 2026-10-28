use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::Sparkline,
};

use super::{Ctx, Panel};
use crate::input::{Action, HitMap};

/// Sparkline compacto por servicio: nombre + valor arriba, tendencia abajo.
pub struct Spark {
    pub title: String,
    pub ids: Vec<usize>,
}

impl Panel for Spark {
    fn title(&self) -> &str {
        &self.title
    }

    fn render(&self, f: &mut Frame, area: Rect, ctx: &Ctx, hits: &mut HitMap) -> usize {
        let t = &ctx.app.theme;
        let ids = ctx.ids(&self.ids);
        let block_h = 1 + ctx.row_h();
        let visible = (area.height / block_h).max(1) as usize;
        let max = ids.len().saturating_sub(visible);

        for (row, &i) in ids
            .iter()
            .skip(ctx.scroll.min(max))
            .take(visible)
            .enumerate()
        {
            // El color sigue al servicio aunque cambie el orden.
            let k = self.ids.iter().position(|&x| x == i).unwrap_or(0);
            let svc = &ctx.app.services[i];
            let y = area.y + row as u16 * block_h;
            if y + 2 > area.bottom() {
                break;
            }
            let [head, line] = Layout::vertical([Constraint::Length(1), Constraint::Length(1)])
                .areas(Rect::new(area.x, y, area.width, 2));
            let up = svc.is_up();
            let color = if up == Some(false) {
                t.err
            } else {
                t.series(k)
            };

            f.render_widget(Span::styled(svc.name.as_str(), t.fg), head);
            f.render_widget(Line::styled(svc.value_label(), color).right_aligned(), head);

            // Una barra por muestra, las más recientes a la derecha; caídas = hueco rojo.
            let n = line.width as usize;
            let mut data: Vec<Option<u64>> = svc
                .history
                .iter()
                .rev()
                .take(n)
                .map(|s| {
                    if s.ok {
                        s.value.map(|v| (v * 10.0).round() as u64)
                    } else {
                        None
                    }
                })
                .collect();
            data.reverse();
            let spark = Sparkline::default()
                .data(data)
                .style(color)
                .absent_value_symbol(ratatui::symbols::bar::ONE_EIGHTH)
                .absent_value_style(t.err);
            f.render_widget(spark, line);

            hits.push(
                Rect::new(area.x, y, area.width, block_h.min(area.bottom() - y)),
                Action::Select(i),
            );
        }
        max
    }
}
