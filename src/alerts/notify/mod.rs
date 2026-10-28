//! Canales de notificación. Para añadir uno: implementa [`Notifier`] en un
//! módulo y regístralo por su `kind` en [`build`].

mod desktop;
mod ntfy;

use anyhow::{Result, bail};

use super::Alert;
use crate::{checks::BoxFut, config::ChannelCfg};

pub trait Notifier: Send + Sync {
    /// Nombre para mostrar en el menú, p. ej. `ntfy · mi-tema`.
    fn label(&self) -> String;

    fn send<'a>(&'a self, alert: &'a Alert) -> BoxFut<'a, Result<()>>;
}

pub fn build(cfg: &ChannelCfg) -> Result<Box<dyn Notifier>> {
    Ok(match cfg.kind.as_str() {
        "desktop" => Box::new(desktop::Desktop),
        "ntfy" => Box::new(ntfy::Ntfy::new(cfg)?),
        other => bail!("canal desconocido {other:?} (desktop, ntfy)"),
    })
}
