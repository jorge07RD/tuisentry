use std::{
    collections::VecDeque,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

/// Hora actual en segundos unix, la escala de `Sample::at`.
pub fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sample {
    /// Segundos unix.
    pub at: f64,
    pub ok: bool,
    /// Latencia en ms o el valor medido por un `cmd`; `None` si no aplica.
    pub value: Option<f64>,
    pub detail: String,
}

/// Historial acotado de muestras de un servicio.
#[derive(Debug, Clone)]
pub struct History {
    buf: VecDeque<Sample>,
    cap: usize,
}

impl History {
    pub fn new(cap: usize) -> Self {
        Self {
            buf: VecDeque::with_capacity(cap),
            cap,
        }
    }

    pub fn push(&mut self, s: Sample) {
        if self.buf.len() == self.cap {
            self.buf.pop_front();
        }
        self.buf.push_back(s);
    }

    pub fn last(&self) -> Option<&Sample> {
        self.buf.back()
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Sample> {
        self.buf.iter()
    }

    pub fn since(&self, t: f64) -> impl Iterator<Item = &Sample> {
        self.buf.iter().filter(move |s| s.at >= t)
    }

    /// Fracción de muestras OK (0..=1); `None` sin datos.
    pub fn uptime(&self) -> Option<f64> {
        if self.buf.is_empty() {
            return None;
        }
        let ok = self.buf.iter().filter(|s| s.ok).count();
        Some(ok as f64 / self.buf.len() as f64)
    }

    /// (mín, media, máx) de los valores registrados.
    pub fn stats(&self) -> Option<(f64, f64, f64)> {
        let vals: Vec<f64> = self.buf.iter().filter_map(|s| s.value).collect();
        if vals.is_empty() {
            return None;
        }
        let min = vals.iter().copied().fold(f64::INFINITY, f64::min);
        let max = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Some((min, vals.iter().sum::<f64>() / vals.len() as f64, max))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(at: f64, ok: bool, v: f64) -> Sample {
        Sample {
            at,
            ok,
            value: Some(v),
            detail: String::new(),
        }
    }

    #[test]
    fn ring_drops_oldest() {
        let mut h = History::new(3);
        for i in 0..5 {
            h.push(s(i as f64, true, i as f64));
        }
        let ats: Vec<f64> = h.iter().map(|s| s.at).collect();
        assert_eq!(ats, [2.0, 3.0, 4.0]);
        assert_eq!(h.last().unwrap().at, 4.0);
    }

    #[test]
    fn uptime_and_stats() {
        let mut h = History::new(10);
        assert!(h.uptime().is_none());
        h.push(s(0.0, true, 10.0));
        h.push(s(1.0, false, 30.0));
        assert_eq!(h.uptime(), Some(0.5));
        assert_eq!(h.stats(), Some((10.0, 20.0, 30.0)));
        assert_eq!(h.since(0.5).count(), 1);
    }
}
