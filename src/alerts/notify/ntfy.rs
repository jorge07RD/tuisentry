use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde_json::json;

use super::Notifier;
use crate::{alerts::Alert, checks::BoxFut, config::ChannelCfg};

/// Push al celular con [ntfy](https://ntfy.sh): instala la app y suscríbete al tema.
pub struct Ntfy {
    server: String,
    topic: String,
    token: Option<String>,
    client: Client,
}

impl Ntfy {
    pub fn new(cfg: &ChannelCfg) -> Result<Self> {
        Ok(Self {
            server: cfg
                .server
                .clone()
                .unwrap_or_else(|| "https://ntfy.sh".into())
                .trim_end_matches('/')
                .into(),
            topic: cfg.topic.clone().context("ntfy necesita `topic`")?,
            token: cfg.token.clone(),
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()?,
        })
    }
}

impl Notifier for Ntfy {
    fn label(&self) -> String {
        format!("ntfy · {}", self.topic)
    }

    fn send<'a>(&'a self, alert: &'a Alert) -> BoxFut<'a, Result<()>> {
        Box::pin(async move {
            // Publicar en JSON admite UTF-8 en el título (las cabeceras no).
            let mut body = json!({
                "topic": self.topic,
                "title": alert.title(),
                "message": alert.body(),
                "priority": alert.priority(),
                "tags": [alert.tag()],
            });
            if let Some(link) = &alert.link {
                // Tocar la notificación abre el recurso.
                body["click"] = json!(link);
            }
            let mut req = self
                .client
                .post(&self.server)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.to_string());
            if let Some(token) = &self.token {
                req = req.bearer_auth(token);
            }
            let resp = req.send().await?;
            if !resp.status().is_success() {
                bail!("ntfy respondió {}", resp.status());
            }
            Ok(())
        })
    }
}
