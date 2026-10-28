//! Alertas: qué merece un aviso y por dónde se manda.
//! Las caídas y recuperaciones vienen de `engine::health`; aquí se suman la
//! lentitud y el certificado por vencer.

pub mod notify;

use crate::{app::fmt_value, store::Sample, time::human};

#[derive(Debug, Clone, PartialEq)]
pub enum AlertKind {
    Down { cause: String },
    Recovered { duration: f64 },
    Slow { ms: f64, threshold: f64 },
    SlowOver,
    CertExpiring { days: i64 },
    Test,
}

#[derive(Debug, Clone)]
pub struct Alert {
    pub service: String,
    pub link: Option<String>,
    pub kind: AlertKind,
}

impl Alert {
    pub fn title(&self) -> String {
        let s = &self.service;
        match &self.kind {
            AlertKind::Down { .. } => format!("✗ {s} caído"),
            AlertKind::Recovered { .. } => format!("✓ {s} recuperado"),
            AlertKind::Slow { .. } => format!("◔ {s} lento"),
            AlertKind::SlowOver => format!("✓ {s} vuelve a ir rápido"),
            AlertKind::CertExpiring { .. } => format!("⚠ certificado de {s} por vencer"),
            AlertKind::Test => "tuisentry: alerta de prueba".into(),
        }
    }

    pub fn body(&self) -> String {
        match &self.kind {
            AlertKind::Down { cause } => cause.clone(),
            AlertKind::Recovered { duration } => format!("estuvo caído {}", human(*duration)),
            AlertKind::Slow { ms, threshold } => format!(
                "{} (umbral {})",
                fmt_value(*ms, "ms"),
                fmt_value(*threshold, "ms")
            ),
            AlertKind::SlowOver => "latencia normal".into(),
            AlertKind::CertExpiring { days } => format!("vence en {days} días"),
            AlertKind::Test => "Si ves esto, los avisos funcionan.".into(),
        }
    }

    /// 1 (mínima) a 5 (urgente), la escala de ntfy.
    pub fn priority(&self) -> u8 {
        match self.kind {
            AlertKind::Down { .. } => 5,
            AlertKind::CertExpiring { .. } => 4,
            AlertKind::Slow { .. } | AlertKind::Recovered { .. } | AlertKind::Test => 3,
            AlertKind::SlowOver => 2,
        }
    }

    /// Etiqueta corta para el icono en ntfy.
    pub fn tag(&self) -> &'static str {
        match self.kind {
            AlertKind::Down { .. } => "rotating_light",
            AlertKind::Recovered { .. } | AlertKind::SlowOver => "white_check_mark",
            AlertKind::Slow { .. } => "turtle",
            AlertKind::CertExpiring { .. } => "lock",
            AlertKind::Test => "bell",
        }
    }
}

/// Estado de lentitud y certificado de un servicio.
#[derive(Debug, Default)]
pub struct Evaluator {
    slow: bool,
    cert_warned: bool,
}

impl Evaluator {
    /// Para salir de "lento", la latencia debe bajar de este fracción del umbral
    /// (evita avisos alternos cuando ronda el límite).
    const HYSTERESIS: f64 = 0.8;

    pub fn observe(&mut self, s: &Sample, slow_ms: Option<u64>, cert_days: i64) -> Vec<AlertKind> {
        let mut out = vec![];
        if let (true, Some(ms), Some(thr)) = (s.ok, s.value, slow_ms) {
            let thr = thr as f64;
            if !self.slow && ms > thr {
                self.slow = true;
                out.push(AlertKind::Slow { ms, threshold: thr });
            } else if self.slow && ms < thr * Self::HYSTERESIS {
                self.slow = false;
                out.push(AlertKind::SlowOver);
            }
        }
        if let Some(days) = s.cert_days {
            if days <= cert_days && !self.cert_warned {
                self.cert_warned = true;
                out.push(AlertKind::CertExpiring { days });
            } else if days > cert_days {
                // Certificado renovado: se puede volver a avisar.
                self.cert_warned = false;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(ms: f64, cert: Option<i64>) -> Sample {
        Sample {
            ok: true,
            value: Some(ms),
            cert_days: cert,
            ..Default::default()
        }
    }

    #[test]
    fn slow_with_hysteresis() {
        let mut e = Evaluator::default();
        assert!(e.observe(&s(100.0, None), Some(500), 14).is_empty());
        assert_eq!(
            e.observe(&s(900.0, None), Some(500), 14),
            [AlertKind::Slow {
                ms: 900.0,
                threshold: 500.0
            }]
        );
        // Sigue lento o apenas bajo el umbral: sin avisos repetidos.
        assert!(e.observe(&s(800.0, None), Some(500), 14).is_empty());
        assert!(e.observe(&s(450.0, None), Some(500), 14).is_empty());
        assert_eq!(
            e.observe(&s(300.0, None), Some(500), 14),
            [AlertKind::SlowOver]
        );
        // Sin umbral nunca avisa.
        assert!(e.observe(&s(9000.0, None), None, 14).is_empty());
    }

    #[test]
    fn cert_warns_once_until_renewed() {
        let mut e = Evaluator::default();
        assert!(e.observe(&s(1.0, Some(60)), None, 14).is_empty());
        assert_eq!(
            e.observe(&s(1.0, Some(10)), None, 14),
            [AlertKind::CertExpiring { days: 10 }]
        );
        assert!(e.observe(&s(1.0, Some(9)), None, 14).is_empty());
        assert!(e.observe(&s(1.0, Some(90)), None, 14).is_empty());
        assert_eq!(
            e.observe(&s(1.0, Some(5)), None, 14),
            [AlertKind::CertExpiring { days: 5 }]
        );
    }
}
