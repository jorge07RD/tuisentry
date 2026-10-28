use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Stylize,
    symbols::Marker,
    text::{Line, Span},
    widgets::{Axis, Chart as RChart, Dataset, GraphType, Paragraph},
};

use super::{Ctx, Panel, fmt_span};
use crate::{
    app::fmt_value,
    input::{Action, HitMap},
};

/// Gráfica de líneas multi-serie sobre una ventana de tiempo.
pub struct Chart {
    pub title: String,
    pub ids: Vec<usize>,
    /// Segundos visibles.
    pub window: f64,
}

impl Panel for Chart {
    fn title(&self) -> &str {
        &self.title
    }

    fn render(&self, f: &mut Frame, area: Rect, ctx: &Ctx, hits: &mut HitMap) -> usize {
        let [legend, body] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);
        draw_legend(f, legend, &self.ids, ctx, hits);
        draw(f, body, &self.ids, ctx.window(self.window), ctx, hits);
        0
    }
}

/// Leyenda tocable: cada nombre abre su detalle.
fn draw_legend(f: &mut Frame, area: Rect, ids: &[usize], ctx: &Ctx, hits: &mut HitMap) {
    let t = &ctx.app.theme;
    let mut x = area.x;
    for (k, &i) in ids.iter().enumerate() {
        let label = format!("━ {}  ", ctx.app.services[i].name);
        let w = Line::raw(label.as_str()).width() as u16;
        if x + w > area.right() {
            break;
        }
        let r = Rect::new(x, area.y, w, 1);
        f.render_widget(Span::styled(label, t.series(k)), r);
        hits.push(r, Action::Select(i));
        x += w;
    }
}

/// Dibuja las series de `ids`; reutilizado por el popup de detalle.
pub fn draw(f: &mut Frame, area: Rect, ids: &[usize], window: f64, ctx: &Ctx, hits: &mut HitMap) {
    let t = &ctx.app.theme;
    let x0 = ctx.now - window;
    let series: Vec<Vec<(f64, f64)>> = ids
        .iter()
        .map(|&i| {
            ctx.app.services[i]
                .history
                .since(x0)
                .filter(|s| s.ok)
                .filter_map(|s| s.value.map(|v| (s.at, v)))
                .collect()
        })
        .collect();
    let downs: Vec<(f64, f64)> = ids
        .iter()
        .flat_map(|&i| {
            ctx.app.services[i]
                .history
                .since(x0)
                .filter(|s| !s.ok)
                .map(|s| (s.at, 0.0))
        })
        .collect();

    if ids.len() == 1 {
        hits.push(area, Action::Select(ids[0]));
    }
    if series.iter().all(Vec::is_empty) && downs.is_empty() {
        let msg = Paragraph::new("esperando datos…").fg(t.dim).centered();
        f.render_widget(
            msg,
            Rect {
                y: area.y + area.height / 2,
                height: 1,
                ..area
            },
        );
        return;
    }

    let peak = series.iter().flatten().map(|p| p.1).fold(0.0, f64::max);
    let ymax = if peak > 0.0 { peak * 1.15 } else { 1.0 };
    let suffix = ids
        .first()
        .map_or("", |&i| ctx.app.services[i].suffix.as_str());

    let mut datasets: Vec<Dataset> = series
        .iter()
        .enumerate()
        .map(|(k, pts)| {
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .fg(t.series(k))
                .data(pts)
        })
        .collect();
    datasets.push(
        Dataset::default()
            .marker(Marker::Dot)
            .graph_type(GraphType::Scatter)
            .fg(t.err)
            .data(&downs),
    );

    let x_labels = [
        format!("-{}", fmt_span(window)),
        format!("-{}", fmt_span(window / 2.0)),
        "ahora".into(),
    ];
    let y_labels = [0.0, ymax / 2.0, ymax].map(|v| fmt_value(v, suffix));
    let chart = RChart::new(datasets)
        .x_axis(
            Axis::default()
                .bounds([x0, ctx.now])
                .labels(x_labels)
                .fg(t.dim),
        )
        .y_axis(
            Axis::default()
                .bounds([0.0, ymax])
                .labels(y_labels)
                .fg(t.dim),
        );
    f.render_widget(chart, area);
}
