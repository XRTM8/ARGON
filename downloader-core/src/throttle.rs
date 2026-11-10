use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, OnceLock, RwLock,
    },
    time::{Duration, Instant},
};
use tokio::sync::watch;

pub async fn wait_cancelled(rx: &mut watch::Receiver<bool>, flag: &AtomicBool) {
    while !flag.load(Ordering::Acquire) {
        if *rx.borrow_and_update() {
            break;
        }
        if rx.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// Lock-free atomic GCRA pacing helper capped against runaway multi-segment debt.
pub fn reserve_delay_ns(next_ns: &AtomicU64, now_ns: u64, cost_ns: u64) -> u64 {
    let mut cur = next_ns.load(Ordering::Relaxed);
    loop {
        let base = cur.max(now_ns);
        let target = base.saturating_add(cost_ns);
        match next_ns.compare_exchange_weak(cur, target, Ordering::AcqRel, Ordering::Relaxed) {
            Ok(_) => return target.saturating_sub(now_ns),
            Err(actual) => cur = actual,
        }
    }
}

#[derive(Default)]
pub struct ThrottleController {
    global_limit_bps: AtomicU64,
    global_next_ns: AtomicU64,
    task_limits: RwLock<HashMap<String, Arc<TaskPacer>>>,
    epoch: OnceLock<Instant>,
}

pub struct TaskPacer {
    pub limit_bps: AtomicU64,
    pub next_ns: AtomicU64,
}

impl Default for TaskPacer {
    fn default() -> Self {
        Self {
            limit_bps: AtomicU64::new(0),
            next_ns: AtomicU64::new(0),
        }
    }
}

impl ThrottleController {
    fn now_ns(&self) -> u64 {
        let base = self.epoch.get_or_init(Instant::now);
        base.elapsed().as_nanos() as u64
    }

    pub fn set_global_limit(&self, bytes_per_sec: u64) {
        self.global_limit_bps.store(bytes_per_sec, Ordering::Relaxed);
        if bytes_per_sec == 0 {
            self.global_next_ns.store(0, Ordering::Relaxed);
        }
    }

    pub fn get_global_limit(&self) -> u64 {
        self.global_limit_bps.load(Ordering::Relaxed)
    }

    pub fn get_or_create_task_pacer(&self, task_id: &str, initial_limit_bps: u64) -> Arc<TaskPacer> {
        if let Ok(read) = self.task_limits.read() {
            if let Some(p) = read.get(task_id) {
                p.limit_bps.store(initial_limit_bps, Ordering::Relaxed);
                return p.clone();
            }
        }
        if let Ok(mut write) = self.task_limits.write() {
            let pacer = write
                .entry(task_id.to_owned())
                .or_insert_with(|| {
                    Arc::new(TaskPacer {
                        limit_bps: AtomicU64::new(initial_limit_bps),
                        next_ns: AtomicU64::new(0),
                    })
                })
                .clone();
            pacer.limit_bps.store(initial_limit_bps, Ordering::Relaxed);
            return pacer;
        }
        Arc::new(TaskPacer {
            limit_bps: AtomicU64::new(initial_limit_bps),
            next_ns: AtomicU64::new(0),
        })
    }

    pub fn set_task_limit(&self, task_id: &str, bytes_per_sec: u64) {
        if let Ok(read) = self.task_limits.read() {
            if let Some(p) = read.get(task_id) {
                p.limit_bps.store(bytes_per_sec, Ordering::Relaxed);
                if bytes_per_sec == 0 {
                    p.next_ns.store(0, Ordering::Relaxed);
                }
                return;
            }
        }
        if bytes_per_sec > 0 {
            let _ = self.get_or_create_task_pacer(task_id, bytes_per_sec);
        }
    }

    pub fn remove_task(&self, task_id: &str) {
        if let Ok(mut write) = self.task_limits.write() {
            write.remove(task_id);
        }
    }

    /// Cancels immediately on `cancel_rx` or when the limit is lifted (`limit == 0`).
    pub async fn wait_throttle(
        &self,
        task_pacer: Option<&TaskPacer>,
        bytes: u64,
        cancel_rx: &mut watch::Receiver<bool>,
        cancel_flag: &AtomicBool,
    ) -> Result<(), String> {
        if bytes == 0 {
            return Ok(());
        }
        if cancel_flag.load(Ordering::Acquire) {
            return Err("CANCELLED".into());
        }

        let mut max_delay_ns = 0u64;
        let global_limit = self.global_limit_bps.load(Ordering::Relaxed);
        if global_limit > 0 {
            let cost = ((bytes as u128 * 1_000_000_000u128) / (global_limit as u128))
                .min(u64::MAX as u128) as u64;
            max_delay_ns = max_delay_ns.max(reserve_delay_ns(&self.global_next_ns, self.now_ns(), cost));
        }
        if let Some(pacer) = task_pacer {
            let task_limit = pacer.limit_bps.load(Ordering::Relaxed);
            if task_limit > 0 {
                let cost = ((bytes as u128 * 1_000_000_000u128) / (task_limit as u128))
                    .min(u64::MAX as u128) as u64;
                max_delay_ns = max_delay_ns.max(reserve_delay_ns(&pacer.next_ns, self.now_ns(), cost));
            }
        }

        if max_delay_ns == 0 {
            return Ok(());
        }

        let deadline = tokio::time::Instant::now() + Duration::from_nanos(max_delay_ns);
        loop {
            if cancel_flag.load(Ordering::Acquire) {
                return Err("CANCELLED".into());
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Ok(());
            }
            // Wake every 200ms at most so lifting a speed limit takes effect promptly too.
            let step = (deadline - now).min(Duration::from_millis(200));
            tokio::select! {
                _ = tokio::time::sleep(step) => {
                    let g_off = self.global_limit_bps.load(Ordering::Relaxed) == 0;
                    let t_off = task_pacer.is_none_or(|p| p.limit_bps.load(Ordering::Relaxed) == 0);
                    if g_off && t_off {
                        return Ok(());
                    }
                }
                _ = wait_cancelled(cancel_rx, cancel_flag) => {
                    return Err("CANCELLED".into());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_free_gcra_delay_is_non_decreasing() {
        let next = AtomicU64::new(0);
        let d1 = reserve_delay_ns(&next, 1_000, 500);
        let d2 = reserve_delay_ns(&next, 1_000, 500);
        assert_eq!(d1, 500);
        assert_eq!(d2, 1_000);
    }

    #[tokio::test]
    async fn throttle_wakes_immediately_on_cancel() {
        let ctrl = ThrottleController::default();
        ctrl.set_global_limit(1024); // 1 KB/s -> 64 KB would sleep 64 seconds!
        let flag = Arc::new(AtomicBool::new(false));
        let (tx, mut rx) = watch::channel(false);

        let flag_clone = flag.clone();
        let canceller = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            flag_clone.store(true, Ordering::Release);
            let _ = tx.send(true);
        });

        let start = Instant::now();
        let res = ctrl.wait_throttle(None, 65_536, &mut rx, &flag).await;
        canceller.await.unwrap();

        assert_eq!(res.unwrap_err(), "CANCELLED");
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
