use super::{BoxFut, Check, Outcome, run_shell};

/// Check genérico: un comando cuyo stdout empieza con un número (se ignora un `%` final).
/// Sale con código ≠ 0 → caído.
pub struct Cmd {
    pub command: String,
    pub ssh: Option<String>,
}

impl Check for Cmd {
    fn run(&self) -> BoxFut<'_, Outcome> {
        Box::pin(async move {
            match run_shell(&self.command, self.ssh.as_deref()).await {
                Ok((true, out)) => Outcome::up(parse_number(&out), first_line(&out)),
                Ok((false, out)) => Outcome::down(first_line(&out)),
                Err(e) => Outcome::down(e.to_string()),
            }
        })
    }

    fn default_suffix(&self) -> &'static str {
        ""
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}

fn parse_number(s: &str) -> Option<f64> {
    s.split_whitespace()
        .next()?
        .trim_end_matches('%')
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::parse_number;

    #[test]
    fn numbers() {
        assert_eq!(parse_number(" 42%\n"), Some(42.0));
        assert_eq!(parse_number("3.5 load"), Some(3.5));
        assert_eq!(parse_number("ok"), None);
    }
}
