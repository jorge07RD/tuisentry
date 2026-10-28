//! Convierte la serie de muestras de un servicio en transiciones de estado,
//! con tolerancia a fallos aislados (`fail_after`).

use crate::store::Sample;

#[derive(Debug, Clone, PartialEq)]
pub enum Transition {
    /// Caído desde la primera falla de la racha.
    Down { since: f64, cause: String },
    /// Recuperado tras `duration` segundos caído.
    Up { since: f64, duration: f64 },
}

#[derive(Debug, Default)]
pub struct Tracker {
    fails: u32,
    first_fail: f64,
    down_since: Option<f64>,
}

impl Tracker {
    #[cfg(test)]
    pub fn is_down(&self) -> bool {
        self.down_since.is_some()
    }

    pub fn observe(&mut self, s: &Sample, fail_after: u32) -> Option<Transition> {
        if s.ok {
            self.fails = 0;
            let since = self.down_since.take()?;
            return Some(Transition::Up {
                since,
                duration: s.at - since,
            });
        }
        if self.fails == 0 {
            self.first_fail = s.at;
        }
        self.fails += 1;
        if self.down_since.is_none() && self.fails >= fail_after.max(1) {
            self.down_since = Some(self.first_fail);
            return Some(Transition::Down {
                since: self.first_fail,
                cause: s.detail.clone(),
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(at: f64, ok: bool) -> Sample {
        Sample {
            at,
            ok,
            value: None,
            detail: if ok { "ok" } else { "timeout" }.into(),
        }
    }

    #[test]
    fn single_failure_is_ignored() {
        let mut t = Tracker::default();
        assert_eq!(t.observe(&s(0.0, false), 2), None);
        assert_eq!(t.observe(&s(1.0, true), 2), None);
        assert!(!t.is_down());
    }

    #[test]
    fn down_after_streak_then_recovers() {
        let mut t = Tracker::default();
        assert_eq!(t.observe(&s(10.0, false), 2), None);
        assert_eq!(
            t.observe(&s(20.0, false), 2),
            Some(Transition::Down {
                since: 10.0,
                cause: "timeout".into()
            })
        );
        // Seguir caído no repite la transición.
        assert_eq!(t.observe(&s(30.0, false), 2), None);
        assert_eq!(
            t.observe(&s(70.0, true), 2),
            Some(Transition::Up {
                since: 10.0,
                duration: 60.0
            })
        );
    }
}
