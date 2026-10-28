use serde::{Deserialize, Serialize};

use crate::app::Service;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Sort {
    /// Orden de la config.
    #[default]
    Config,
    Name,
    /// Caídos primero.
    Status,
    /// Más lentos primero.
    Latency,
}

impl Sort {
    pub const ALL: [Sort; 4] = [Sort::Config, Sort::Name, Sort::Status, Sort::Latency];

    pub fn label(self) -> &'static str {
        match self {
            Sort::Config => "config",
            Sort::Name => "nombre",
            Sort::Status => "estado",
            Sort::Latency => "latencia",
        }
    }
}

/// Cómo se muestran los paneles; se cambia desde el menú y se recuerda.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct View {
    /// Ventana de las gráficas en segundos; `None` = la de cada panel.
    pub window: Option<f64>,
    pub down_only: bool,
    pub sort: Sort,
    /// Filas de 2 líneas para el dedo.
    pub touch: bool,
}

impl View {
    pub fn new(web: bool) -> Self {
        Self {
            window: None,
            down_only: false,
            sort: Sort::Config,
            touch: web,
        }
    }

    pub fn row_h(&self) -> u16 {
        if self.touch { 2 } else { 1 }
    }

    /// Filtra y ordena los índices de servicio de un panel.
    pub fn apply(&self, ids: &[usize], services: &[Service]) -> Vec<usize> {
        let mut out: Vec<usize> = ids
            .iter()
            .copied()
            .filter(|&i| !self.down_only || services[i].is_up() == Some(false))
            .collect();
        match self.sort {
            Sort::Config => {}
            Sort::Name => out.sort_by_key(|&i| services[i].name.to_lowercase()),
            Sort::Status => out.sort_by_key(|&i| match services[i].is_up() {
                Some(false) => 0,
                None => 1,
                Some(true) => 2,
            }),
            Sort::Latency => out.sort_by(|&a, &b| {
                let v = |i: usize| services[i].history.last().and_then(|s| s.value);
                v(b).partial_cmp(&v(a)).unwrap_or(std::cmp::Ordering::Equal)
            }),
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Sample;

    fn svc(name: &str, ok: Option<bool>, v: f64) -> Service {
        let mut s = Service::new(name);
        if let Some(ok) = ok {
            s.history.push(Sample {
                at: 0.0,
                ok,
                value: Some(v),
                detail: String::new(),
            });
        }
        s
    }

    #[test]
    fn filter_and_sort() {
        let services = vec![
            svc("b", Some(true), 10.0),
            svc("a", Some(false), 5.0),
            svc("c", None, 0.0),
            svc("d", Some(true), 90.0),
        ];
        let ids = [0, 1, 2, 3];
        let mut v = View::new(false);
        assert_eq!(v.apply(&ids, &services), [0, 1, 2, 3]);
        v.sort = Sort::Name;
        assert_eq!(v.apply(&ids, &services), [1, 0, 2, 3]);
        v.sort = Sort::Status;
        assert_eq!(v.apply(&ids, &services), [1, 2, 0, 3]);
        v.sort = Sort::Latency;
        assert_eq!(v.apply(&ids, &services)[0], 3);
        v.down_only = true;
        assert_eq!(v.apply(&ids, &services), [1]);
    }
}
