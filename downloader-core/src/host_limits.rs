use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostLimitRecord {
    pub host: String,
    pub max_connections: u32,
}

#[derive(Debug, Clone)]
struct HostEntry {
    learned_limit: usize,
    recorded_at: Instant,
}

/// Adaptive per-host connection limiter that remembers servers that reject extra parallel connections.
pub struct HostLimits {
    inner: Mutex<HashMap<String, HostEntry>>,
    ttl: Duration,
}

impl Default for HostLimits {
    fn default() -> Self {
        Self::new(Duration::from_secs(30 * 60))
    }
}

impl HostLimits {
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            ttl,
        }
    }

    pub fn host_key(raw_url: &str) -> Option<String> {
        let parsed = reqwest::Url::parse(raw_url).ok()?;
        let host = parsed.host_str()?.to_ascii_lowercase();
        let port = parsed.port();
        Some(match port {
            Some(p) => format!("{host}:{p}"),
            None => host,
        })
    }

    /// Returns the effective connection cap for `host`, bounded by `configured_cap`.
    pub fn effective_limit(&self, host: &str, configured_cap: usize) -> usize {
        let cap = configured_cap.max(1);
        let mut guard = self.inner.lock().expect("host limits mutex poisoned");
        if let Some(entry) = guard.get(host) {
            if entry.recorded_at.elapsed() < self.ttl {
                return entry.learned_limit.max(1).min(cap);
            }
            guard.remove(host);
        }
        cap
    }

    /// Records a connection refusal (429 / 503 / 403 / reset) and lowers the host's cap.
    pub fn record_refusal(&self, host: &str, active_connections: usize) {
        let new_limit = active_connections.saturating_sub(1).max(1);
        let mut guard = self.inner.lock().expect("host limits mutex poisoned");
        let entry = guard.entry(host.to_string()).or_insert(HostEntry {
            learned_limit: new_limit,
            recorded_at: Instant::now(),
        });
        if entry.recorded_at.elapsed() >= self.ttl || new_limit < entry.learned_limit {
            entry.learned_limit = new_limit;
            entry.recorded_at = Instant::now();
        }
    }

    pub fn export_limits(&self) -> Vec<HostLimitRecord> {
        let guard = self.inner.lock().expect("host limits mutex poisoned");
        guard
            .iter()
            .filter(|(_, e)| e.recorded_at.elapsed() < self.ttl)
            .map(|(host, e)| HostLimitRecord {
                host: host.clone(),
                max_connections: e.learned_limit as u32,
            })
            .collect()
    }

    pub fn import_limits(&self, records: Vec<HostLimitRecord>) {
        let mut guard = self.inner.lock().expect("host limits mutex poisoned");
        let now = Instant::now();
        for rec in records {
            if rec.max_connections > 0 {
                guard.insert(
                    rec.host,
                    HostEntry {
                        learned_limit: rec.max_connections as usize,
                        recorded_at: now,
                    },
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowers_and_expires_host_limit() {
        let limits = HostLimits::new(Duration::from_millis(50));
        assert_eq!(limits.effective_limit("example.com", 8), 8);
        limits.record_refusal("example.com", 4);
        assert_eq!(limits.effective_limit("example.com", 8), 3);
        std::thread::sleep(Duration::from_millis(70));
        assert_eq!(limits.effective_limit("example.com", 8), 8);
    }

    #[test]
    fn extracts_host_key_with_optional_port() {
        assert_eq!(
            HostLimits::host_key("https://CDN.Example.COM:8443/file.zip").as_deref(),
            Some("cdn.example.com:8443")
        );
        assert_eq!(
            HostLimits::host_key("https://example.com/file.zip").as_deref(),
            Some("example.com")
        );
    }
}
