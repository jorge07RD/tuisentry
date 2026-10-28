use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use reqwest::{Client, tls::TlsInfo};

use super::{BoxFut, Check, Outcome, ms};

pub struct Http {
    url: String,
    expect: Option<u16>,
    client: Client,
}

impl Http {
    pub fn new(url: String, expect: Option<u16>) -> Result<Self> {
        let client = Client::builder()
            .tls_info(true)
            .user_agent(concat!("tuisentry/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            url,
            expect,
            client,
        })
    }
}

impl Check for Http {
    fn run(&self) -> BoxFut<'_, Outcome> {
        Box::pin(async move {
            let t = Instant::now();
            let resp = match self.client.get(&self.url).send().await {
                Ok(r) => r,
                Err(e) => return Outcome::down(root_cause(&e)),
            };
            let lat = ms(t.elapsed());
            let code = resp.status().as_u16();
            let ok = match self.expect {
                Some(want) => code == want,
                None => code < 400,
            };
            let mut detail = format!("HTTP {code} · {lat:.0} ms");
            if let Some(days) = resp
                .extensions()
                .get::<TlsInfo>()
                .and_then(|i| i.peer_certificate())
                .and_then(cert_days_left)
            {
                detail.push_str(&format!(" · cert {days}d"));
            }
            if ok {
                Outcome::up(Some(lat), detail)
            } else {
                Outcome {
                    ok,
                    value: Some(lat),
                    detail,
                }
            }
        })
    }

    fn default_link(&self) -> Option<String> {
        Some(self.url.clone())
    }
}

/// Días hasta que vence el certificado del servidor.
fn cert_days_left(der: &[u8]) -> Option<i64> {
    let (_, cert) = x509_parser::parse_x509_certificate(der).ok()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    Some((cert.validity().not_after.timestamp() - now) / 86_400)
}

/// reqwest anida la causa real (DNS, TLS, conexión rechazada) en `source()`.
fn root_cause(e: &dyn std::error::Error) -> String {
    let mut cur = e;
    while let Some(next) = cur.source() {
        cur = next;
    }
    cur.to_string()
}
