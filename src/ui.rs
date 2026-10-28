use std::time::Duration;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Position, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, Paragraph},
};

use crate::{
    app::{App, fmt_value},
    input::{Action, HitMap, MENU_SCROLL},
    menu::{self, MenuState, Screen},
    panels::{Ctx, chart},
};

/// Por debajo de este ancho se muestra un panel a la vez (celular).
const NARROW: u16 = 100;
const TOAST: Duration = Duration::from_secs(5);

pub fn draw(f: &mut Frame, app: &App, hits: &mut HitMap) {
    let area = f.area();
    // En modo web las barras son de 2 filas: más área para el dedo.
    let bar = if app.web { 2 } else { 1 };
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(bar),
        Constraint::Fill(1),
        Constraint::Length(bar),
    ])
    .areas(area);

    draw_header(f, header, app, hits);
    draw_body(f, body, app, hits);
    draw_footer(f, footer, app, hits);

    if let Some(i) = app.selected {
        draw_detail(f, app, i, hits);
    }
    if app.help {
        draw_help(f, app, hits);
    }
    if let Some(m) = &app.menu {
        draw_menu(f, app, m, hits);
    }
    draw_toast(f, body, app);
}

fn draw_header(f: &mut Frame, band: Rect, app: &App, hits: &mut HitMap) {
    let t = &app.theme;
    let row = Rect { height: 1, ..band };
    let narrow = band.width < NARROW;
    let menu_btn = Rect::new(row.x, row.y, 3, 1);
    f.render_widget(
        Span::styled(" ≡ ", Style::new().fg(t.fg).bg(t.surface).bold()),
        menu_btn,
    );
    hits.push(
        Rect {
            height: band.height,
            ..menu_btn
        },
        Action::Menu,
    );
    let brand = if narrow { " ◆ " } else { " ◆ tuisentry " };
    f.render_widget(
        Span::styled(brand, Style::new().fg(t.accent).bold()),
        Rect {
            x: row.x + 3,
            width: row.width.saturating_sub(3),
            ..row
        },
    );
    let brand = format!("≡ {brand}");

    let up = app
        .services
        .iter()
        .filter(|s| s.is_up() == Some(true))
        .count();
    let down = app
        .services
        .iter()
        .filter(|s| s.is_up() == Some(false))
        .count();
    let mut summary = vec![
        Span::styled(format!("● {up} "), t.ok),
        Span::styled(format!("● {down} "), if down > 0 { t.err } else { t.dim }),
    ];
    if !app.mouse_wanted() {
        summary.insert(0, Span::styled("⇱ mouse libre  ", t.warn));
    }
    let summary = Line::from(summary).right_aligned();
    let summary_w = summary.width() as u16;
    f.render_widget(summary, row);

    let mut x = row.x + Line::raw(brand.as_str()).width() as u16 + 1;
    let limit = row.right().saturating_sub(summary_w + 1);
    for (i, page) in app.pages.iter().enumerate() {
        let label = if narrow {
            format!(" {} ", page.title)
        } else {
            format!(" {} {} ", i + 1, page.title)
        };
        let w = Line::raw(label.as_str()).width() as u16;
        if x + w > limit {
            break;
        }
        let style = if i == app.page {
            Style::new().fg(t.surface).bg(t.accent).bold()
        } else {
            Style::new().fg(t.dim)
        };
        f.render_widget(Span::styled(label, style), Rect::new(x, row.y, w, 1));
        hits.push(Rect::new(x, band.y, w, band.height), Action::Page(i));
        x += w + 1;
    }
}

fn draw_body(f: &mut Frame, area: Rect, app: &App, hits: &mut HitMap) {
    let page = app.page();
    let n = page.panels.len();
    if n == 0 {
        let msg = Paragraph::new("Esta página no tiene paneles")
            .fg(app.theme.dim)
            .centered();
        f.render_widget(msg, area);
        return;
    }
    if let Some(z) = app.zoom.filter(|&z| z < n) {
        return draw_panel(f, area, app, z, hits);
    }
    if area.width < NARROW {
        return draw_panel(f, area, app, page.focus.min(n - 1), hits);
    }

    let cols = match n {
        1 => 1,
        2..=4 => 2,
        _ => 3,
    };
    let rows = n.div_ceil(cols);
    let row_areas = Layout::vertical(vec![Constraint::Fill(1); rows]).split(area);
    for (r, row_area) in row_areas.iter().enumerate() {
        // La última fila reparte su ancho si le sobran huecos.
        let in_row = (n - r * cols).min(cols);
        let cells = Layout::horizontal(vec![Constraint::Fill(1); in_row]).split(*row_area);
        for (c, cell) in cells.iter().enumerate() {
            draw_panel(f, *cell, app, r * cols + c, hits);
        }
    }
}

fn draw_panel(f: &mut Frame, area: Rect, app: &App, idx: usize, hits: &mut HitMap) {
    let t = &app.theme;
    let page = app.page();
    let panel = &page.panels[idx];
    let focused = page.focus == idx;
    let zoomed = app.zoom == Some(idx);

    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            panel.title(),
            Style::new()
                .fg(if focused { t.accent } else { t.fg })
                .bold(),
        ),
        Span::raw(" "),
    ]);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(if focused { t.accent } else { t.border })
        .title(title)
        .title(Line::styled(if zoomed { " ⤡ " } else { " ⤢ " }, t.dim).right_aligned());
    let inner = block.inner(area).inner(Margin::new(1, 0));
    f.render_widget(block, area);
    // Tocar la barra de título hace zoom (o lo quita).
    hits.push(Rect { height: 1, ..area }, Action::Zoom(idx));

    let ctx = Ctx {
        app,
        now: app.now(),
        scroll: page.scroll[idx],
    };
    let max = panel.render(f, inner, &ctx, hits);
    hits.scroll(inner, idx, max);
}

fn draw_footer(f: &mut Frame, band: Rect, app: &App, hits: &mut HitMap) {
    let t = &app.theme;
    let row = Rect {
        y: band.bottom() - 1,
        height: 1,
        ..band
    };
    let narrow = band.width < NARROW;
    let btn = Style::new().fg(t.fg).bg(t.surface);

    let buttons: &[(&str, Action)] = if narrow {
        &[
            (" ⟳ ", Action::Check(None)),
            (" ? ", Action::Help),
            (" ✕ ", Action::Quit),
        ]
    } else {
        &[
            (" ⟳ check ", Action::Check(None)),
            (" ? ayuda ", Action::Help),
            (" ✕ salir ", Action::Quit),
        ]
    };
    let mut x = row.right();
    for (label, action) in buttons.iter().rev() {
        let w = Line::raw(*label).width() as u16;
        x = x.saturating_sub(w);
        f.render_widget(Span::styled(*label, btn), Rect::new(x, row.y, w, 1));
        hits.push(Rect::new(x, band.y, w, band.height), *action);
        x = x.saturating_sub(1);
    }
    let left = Rect {
        width: x.saturating_sub(row.x),
        ..row
    };

    let page = app.page();
    if narrow && page.panels.len() > 1 {
        // Navegación entre paneles: ◄ título k/n ►
        let [prev, mid, next] = Layout::horizontal([
            Constraint::Length(5),
            Constraint::Fill(1),
            Constraint::Length(5),
        ])
        .areas(left);
        f.render_widget(Line::styled(" ◄ ", btn), prev);
        f.render_widget(Line::styled(" ► ", btn).right_aligned(), next);
        let focus = app.zoom.unwrap_or(page.focus);
        let label = format!(
            "{} {}/{}",
            page.panels[focus].title(),
            focus + 1,
            page.panels.len()
        );
        f.render_widget(Line::styled(label, t.dim).centered(), mid);
        hits.push(
            Rect {
                y: band.y,
                height: band.height,
                ..prev
            },
            Action::PrevPanel,
        );
        hits.push(
            Rect {
                y: band.y,
                height: band.height,
                ..next
            },
            Action::NextPanel,
        );
    } else if !narrow {
        let hint = " toca un servicio: detalle · título: zoom · rueda/arrastre: scroll";
        f.render_widget(Span::styled(hint, t.dim), left);
    }
}

fn draw_toast(f: &mut Frame, body: Rect, app: &App) {
    let Some((msg, at)) = &app.toast else { return };
    if at.elapsed() > TOAST || body.height < 3 {
        return;
    }
    let t = &app.theme;
    let text = format!("  {msg}  ");
    let w = (Line::raw(text.as_str()).width() as u16).min(body.width);
    let r = Rect::new(body.x + (body.width - w) / 2, body.bottom() - 2, w, 1);
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(text).style(Style::new().fg(t.fg).bg(t.surface)),
        r,
    );
}

fn popup(screen: Rect, max_w: u16, max_h: u16) -> Rect {
    let w = screen.width.saturating_sub(2).min(max_w);
    let h = screen.height.saturating_sub(2).min(max_h);
    Rect::new(
        screen.x + (screen.width - w) / 2,
        screen.y + (screen.height - h) / 2,
        w,
        h,
    )
}

fn card<'a>(app: &App, title: Line<'a>) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(app.theme.accent)
        .style(Style::new().bg(app.theme.surface))
        .title(title)
}

/// Fila de botones; devuelve nada porque registra sus propios hits.
fn button_row(f: &mut Frame, row: Rect, app: &App, buttons: &[(&str, Action)], hits: &mut HitMap) {
    let style = Style::new()
        .fg(app.theme.surface)
        .bg(app.theme.accent)
        .bold();
    let mut x = row.x;
    for (label, action) in buttons {
        let w = Line::raw(*label).width() as u16;
        if x + w > row.right() {
            break;
        }
        let r = Rect::new(x, row.y, w, 1);
        f.render_widget(Span::styled(*label, style), r);
        hits.push(
            Rect {
                height: row.height,
                ..r
            },
            *action,
        );
        x += w + 2;
    }
}

fn draw_detail(f: &mut Frame, app: &App, i: usize, hits: &mut HitMap) {
    let t = &app.theme;
    let svc = &app.services[i];
    let screen = f.area();
    let area = popup(screen, 78, 24);
    hits.modal(screen, area);
    f.render_widget(Clear, area);

    let (dot, color) = match svc.is_up() {
        Some(true) => ("●", t.ok),
        Some(false) => ("●", t.err),
        None => ("○", t.dim),
    };
    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled(dot, color),
        Span::styled(format!(" {} ", svc.name), Style::new().fg(t.fg).bold()),
        Span::styled(format!("{} ", svc.kind), t.dim),
    ]);
    let block = card(app, title);
    let inner = block.inner(area).inner(Margin::new(1, 0));
    f.render_widget(block, area);

    let btn_h = if app.web { 2 } else { 1 };
    let [info, chart_area, buttons] = Layout::vertical([
        Constraint::Length(6),
        Constraint::Fill(1),
        Constraint::Length(btn_h),
    ])
    .areas(inner);

    let key = |k: &str| Span::styled(format!("{k:<9}"), t.dim);
    let last = svc.history.last();
    let mut lines = vec![
        Line::from(vec![
            key("estado"),
            Span::styled(
                last.map_or("esperando…".into(), |s| s.detail.clone()),
                color,
            ),
        ]),
        Line::from(vec![
            key("destino"),
            Span::styled(svc.target.clone().unwrap_or_default(), t.fg),
            Span::styled(
                svc.host
                    .as_ref()
                    .map_or(String::new(), |h| format!("  @{h}")),
                t.dim,
            ),
        ]),
    ];
    if let Some(u) = svc.history.uptime() {
        let n = svc.history.len();
        lines.push(Line::from(vec![
            key("uptime"),
            Span::styled(format!("{:.2}%", u * 100.0), t.fg),
            Span::styled(format!(" · {n} muestras"), t.dim),
        ]));
    }
    if let Some((min, avg, max)) = svc.history.stats() {
        let v = |x| fmt_value(x, &svc.suffix);
        lines.push(Line::from(vec![
            key("valor"),
            Span::styled(
                format!("mín {} · media {} · máx {}", v(min), v(avg), v(max)),
                t.fg,
            ),
        ]));
    }
    let link_row = info.y + lines.len() as u16;
    f.render_widget(Paragraph::new(lines), info);

    if let Some(url) = &svc.link
        && link_row < info.bottom()
    {
        f.render_widget(key("link"), Rect::new(info.x, link_row, 9, 1));
        let avail = info.width.saturating_sub(9) as usize;
        let shown: String = if url.chars().count() > avail {
            url.chars()
                .take(avail.saturating_sub(1))
                .chain(['…'])
                .collect()
        } else {
            url.clone()
        };
        let r = Rect::new(
            info.x + 9,
            link_row,
            Line::raw(shown.as_str()).width() as u16,
            1,
        );
        f.render_widget(
            Span::styled(shown.as_str(), Style::new().fg(t.accent).underlined()),
            r,
        );
        hits.push(r, Action::OpenLink(i));
        hits.link(Position::new(r.x, r.y), &shown, url, t.accent);
    }

    let ctx = Ctx {
        app,
        now: app.now(),
        scroll: 0,
    };
    chart::draw(f, chart_area, &[i], ctx.window(15.0 * 60.0), &ctx, hits);

    let mut row: Vec<(&str, Action)> = vec![];
    if svc.link.is_some() {
        row.push((" ↗ Abrir ", Action::OpenLink(i)));
    }
    row.push((" ⟳ Check ", Action::Check(Some(i))));
    if app.web {
        row.push((" ⇱ Mouse ", Action::FreeMouse));
    }
    row.push((" ⋯ ", Action::MenuGo(Screen::Service(i))));
    row.push((" ✕ ", Action::Close));
    button_row(f, buttons, app, &row, hits);
}

/// Cajón lateral en pantallas anchas; hoja a pantalla completa en el celular
/// o en modo táctil, con filas más altas.
fn draw_menu(f: &mut Frame, app: &App, m: &MenuState, hits: &mut HitMap) {
    let t = &app.theme;
    let screen = f.area();
    let full = screen.width < NARROW || app.view.touch;
    let area = if full {
        screen
    } else {
        Rect {
            width: screen.width.min(36),
            ..screen
        }
    };
    hits.modal(screen, area);
    f.render_widget(Clear, area);

    let at_root = m.stack.is_empty() && m.query.is_empty();
    let heading = if !m.query.is_empty() {
        " ◄ Resultados ".to_string()
    } else if at_root {
        " ≡ Menú ".to_string()
    } else {
        format!(" ◄ {} ", menu::title(m.screen(), app))
    };
    let block = card(app, Line::styled(heading, Style::new().fg(t.fg).bold()))
        .title(Line::styled(" ✕ ", t.dim).right_aligned());
    let inner = block.inner(area).inner(Margin::new(1, 0));
    f.render_widget(block, area);
    // Barra de título: tocar a la izquierda vuelve, la ✕ cierra.
    let top = Rect { height: 1, ..area };
    hits.push(
        top,
        if at_root {
            Action::Close
        } else {
            Action::MenuBack
        },
    );
    hits.push(
        Rect::new(top.right().saturating_sub(4), top.y, 4, 1),
        Action::Close,
    );

    let row_h: u16 = if full { 2 } else { app.view.row_h() };
    let [search, list] =
        Layout::vertical([Constraint::Length(row_h), Constraint::Fill(1)]).areas(inner);
    let query = if m.query.is_empty() {
        Span::styled("buscar…", t.dim)
    } else {
        Span::styled(format!("{}▏", m.query), t.fg)
    };
    f.render_widget(
        Line::from(vec![Span::styled("⌕ ", t.accent), query]),
        Rect {
            height: 1,
            ..search
        },
    );

    let items = m.items(app);
    if items.is_empty() {
        f.render_widget(Line::styled("sin resultados", t.dim), list);
    }
    let visible = (list.height / row_h).max(1) as usize;
    hits.menu_rows = visible;
    let max = items.len().saturating_sub(visible);
    let scroll = m.scroll.min(max);
    for (row, (k, item)) in items
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible)
        .enumerate()
    {
        let y = list.y + row as u16 * row_h;
        let line = Rect::new(list.x, y, list.width, 1);
        if k == m.sel {
            f.buffer_mut().set_style(line, Style::new().bg(t.border));
        }
        let icon_color = match item.icon {
            "✗" => t.err,
            "●" => t.ok,
            _ => t.accent,
        };
        let right = match (item.on, &item.badge) {
            (Some(true), _) => Span::styled("● sí", t.ok),
            (Some(false), _) => Span::styled("○ no", t.dim),
            (None, Some(b)) if item.is_submenu() => Span::styled(format!("{b} ▸"), t.dim),
            (None, Some(b)) => Span::styled(b.as_str(), t.dim),
            (None, None) if item.is_submenu() => Span::styled("▸", t.dim),
            _ => Span::raw(""),
        };
        // La etiqueta cede espacio al valor de la derecha.
        let room = (line.width as usize).saturating_sub(right.width() + 3);
        f.render_widget(
            Line::from(vec![
                Span::styled(format!("{} ", item.icon), icon_color),
                Span::styled(truncate(&item.label, room), t.fg),
            ]),
            line,
        );
        f.render_widget(Line::from(right).right_aligned(), line);
        hits.push(
            Rect::new(list.x, y, list.width, row_h.min(list.bottom() - y)),
            item.action,
        );
    }
    hits.scroll(list, MENU_SCROLL, max);
}

fn draw_help(f: &mut Frame, app: &App, hits: &mut HitMap) {
    let t = &app.theme;
    let screen = f.area();
    let area = popup(screen, 64, 22);
    hits.modal(screen, area);
    f.render_widget(Clear, area);
    let block = card(app, Line::styled(" Ayuda ", Style::new().fg(t.fg).bold()));
    let inner = block.inner(area).inner(Margin::new(1, 0));
    f.render_widget(block, area);

    let item = |k: &'static str, d: &'static str| {
        Line::from(vec![
            Span::styled(format!("{k:<14}"), t.accent),
            Span::styled(d, t.fg),
        ])
    };
    let lines = vec![
        Line::styled("Toque / mouse", Style::new().fg(t.dim).bold()),
        item("≡", "menú: servicios, vista, ajustes, búsqueda"),
        item("servicio", "abre el detalle con gráfica y link"),
        item("↗", "abre el recurso monitoreado"),
        item("título panel", "zoom a pantalla completa"),
        item("pestañas", "cambia de página"),
        item("◄ ►", "panel anterior / siguiente (celular)"),
        item("arrastrar", "desplaza la lista"),
        item("fuera", "cierra el popup"),
        Line::raw(""),
        Line::styled("Teclado", Style::new().fg(t.dim).bold()),
        item("Espacio  :", "menú (escribe para buscar)"),
        item("Tab 1-9", "páginas"),
        item("h l  ← →", "panel anterior / siguiente"),
        item("j k  ↑ ↓", "scroll · Enter/z zoom"),
        item("r  o  m", "re-check · abrir link · mouse libre"),
        item("Esc  q", "cerrar · salir"),
    ];
    let [text, buttons] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(inner);
    f.render_widget(Paragraph::new(lines), text);
    button_row(f, buttons, app, &[(" ✕ Cerrar ", Action::Close)], hits);
}

/// Recorta a `max` columnas con `…` al final.
fn truncate(s: &str, max: usize) -> String {
    if Line::raw(s).width() <= max {
        return s.into();
    }
    let mut out = String::new();
    for c in s.chars() {
        if Line::raw(out.as_str()).width() + 2 > max {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}
