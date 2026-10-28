use anyhow::{Result, bail};

use super::Notifier;
use crate::{alerts::Alert, checks::BoxFut};

/// Notificación de escritorio vía `notify-send` (libnotify).
pub struct Desktop;

impl Notifier for Desktop {
    fn label(&self) -> String {
        "escritorio".into()
    }

    fn send<'a>(&'a self, alert: &'a Alert) -> BoxFut<'a, Result<()>> {
        Box::pin(async move {
            let urgency = match alert.priority() {
                5 => "critical",
                1 | 2 => "low",
                _ => "normal",
            };
            let status = tokio::process::Command::new("notify-send")
                .args(["-a", "tuisentry", "-u", urgency])
                .arg(alert.title())
                .arg(alert.body())
                .status()
                .await?;
            if !status.success() {
                bail!("notify-send salió con {status}");
            }
            Ok(())
        })
    }
}
