//! Tipos de panel. Para añadir uno nuevo: implementa [`Panel`] en un módulo
//! y regístralo por su `kind` en [`build`].

pub mod chart;
mod gauge;
mod spark;
mod status;

use anyhow::{Result, bail};
use ratatui::{Frame, layout::Rect};

use crate::{
    app::{App, Service},
    config::{PanelCfg, parse_duration},
    input::HitMap,
};

pub struct Ctx<'a> {
    pub app: &'a App,
    /// Segundos unix.
    pub now: f64,
    /// Desplazamiento pedido por el usuario; el panel lo acota.
    pub scroll: usize,
}

impl Ctx<'_> {
    /// Servicios del panel tras aplicar el filtro y el orden de la vista.
    pub fn ids(&self, ids: &[usize]) -> Vec<usize> {
        self.app.view.apply(ids, &self.app.services)
    }

    /// Alto de fila: 2 en modo táctil para que el dedo acierte.
    pub fn row_h(&self) -> u16 {
        self.app.view.row_h()
    }

    /// Ventana de la vista si está fijada; si no, la del panel.
    pub fn window(&self, panel: f64) -> f64 {
        self.app.view.window.unwrap_or(panel)
    }
}

pub trait Panel {
    fn title(&self) -> &str;

    /// Dibuja dentro de `area` (sin bordes) y devuelve el scroll máximo (0 = no hay).
    fn render(&self, f: &mut Frame, area: Rect, ctx: &Ctx, hits: &mut HitMap) -> usize;
}

/// Registro de tipos de panel por `kind`.
pub fn build(cfg: &PanelCfg, services: &[Service]) -> Result<Box<dyn Panel>> {
    let numeric = matches!(cfg.kind.as_str(), "chart" | "spark");
    let ids = resolve(cfg, services, numeric);
    let title = |default: &str| cfg.title.clone().unwrap_or_else(|| default.into());
    let window = match &cfg.window {
        Some(w) => parse_duration(w)?.as_secs_f64(),
        None => 15.0 * 60.0,
    };
    Ok(match cfg.kind.as_str() {
        "status" => Box::new(status::Status {
            title: title("Servicios"),
            ids,
        }),
        "chart" => Box::new(chart::Chart {
            title: title("Latencia"),
            ids,
            window,
        }),
        "spark" => Box::new(spark::Spark {
            title: title("Tendencia"),
            ids,
        }),
        "gauge" => Box::new(gauge::Gauge {
            title: title("Uptime"),
            ids,
        }),
        other => bail!("panel desconocido {other:?} (status, chart, spark, gauge)"),
    })
}

/// Índices de servicio del panel: los listados, los del host o todos.
/// Los paneles numéricos omiten por defecto los checks sin valor (systemd).
fn resolve(cfg: &PanelCfg, services: &[Service], numeric: bool) -> Vec<usize> {
    if !cfg.services.is_empty() {
        return cfg
            .services
            .iter()
            .filter_map(|n| services.iter().position(|s| &s.name == n))
            .collect();
    }
    services
        .iter()
        .enumerate()
        .filter(|(_, s)| cfg.host.is_none() || s.host == cfg.host)
        .filter(|(_, s)| !numeric || s.kind != "systemd")
        .map(|(i, _)| i)
        .collect()
}

/// `900` → `15m`.
pub fn fmt_span(secs: f64) -> String {
    let s = secs.round() as u64;
    match s {
        s if s >= 3600 && s % 3600 == 0 => format!("{}h", s / 3600),
        s if s >= 60 => format!("{}m", s / 60),
        s => format!("{s}s"),
    }
}
