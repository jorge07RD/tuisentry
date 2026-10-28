use super::{BoxFut, Check, Outcome, run_shell};

pub struct Systemd {
    pub unit: String,
    pub ssh: Option<String>,
}

impl Check for Systemd {
    fn run(&self) -> BoxFut<'_, Outcome> {
        Box::pin(async move {
            let cmd = format!("systemctl is-active {}", self.unit);
            match run_shell(&cmd, self.ssh.as_deref()).await {
                // `is-active` sale con 0 solo si la unidad está "active".
                Ok((true, state)) => Outcome::up(None, state),
                Ok((false, state)) if state.is_empty() => Outcome::down("inactive"),
                Ok((false, state)) => Outcome::down(state),
                Err(e) => Outcome::down(e.to_string()),
            }
        })
    }

    fn default_suffix(&self) -> &'static str {
        ""
    }
}
