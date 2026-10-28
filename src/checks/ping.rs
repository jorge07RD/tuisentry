use std::net::IpAddr;

use super::{BoxFut, Check, Outcome, ms};

/// ICMP echo sin root (socket DGRAM). Requiere que `net.ipv4.ping_group_range`
/// incluya tu grupo, que es lo habitual en distros con systemd.
pub struct Ping {
    pub host: String,
}

impl Check for Ping {
    fn run(&self) -> BoxFut<'_, Outcome> {
        Box::pin(async move {
            let ip: IpAddr = match self.host.parse() {
                Ok(ip) => ip,
                Err(_) => match tokio::net::lookup_host((self.host.as_str(), 0)).await {
                    Ok(mut addrs) => match addrs.next() {
                        Some(a) => a.ip(),
                        None => return Outcome::down("DNS sin resultados"),
                    },
                    Err(e) => return Outcome::down(format!("DNS: {e}")),
                },
            };
            match surge_ping::ping(ip, &[0; 32]).await {
                Ok((_, rtt)) => {
                    let lat = ms(rtt);
                    Outcome::up(Some(lat), format!("{ip} · {lat:.1} ms"))
                }
                Err(e) => Outcome::down(format!("icmp: {e}")),
            }
        })
    }
}
