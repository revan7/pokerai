//! §12 startup diagnostics, rendered by plan 5's settings panel. Plan 3 fills `quarantined_bundles`,
//! plan 4 fills `cache_state`; both go through `Engine::startup_report`, never through a second channel.
use crate::worker::ready::cpu_lacks_avx2;
use proto::worker::Ready;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StartupReport {
    pub worker_ready: bool,
    pub worker_threads: u8,
    pub build_features: Vec<String>,
    pub cpu_features: Vec<String>,
    pub capabilities: Vec<String>,
    /// §3.7: the build requires AVX2; a CPU without it gets a startup banner rather than a refusal.
    pub cpu_lacks_avx2: bool,
    /// Preflop bundles that failed validation and were quarantined (§8.2); filled by plan 3.
    pub quarantined_bundles: Vec<String>,
    /// "absent" until plan 4 opens the cache, then its own summary.
    pub cache_state: String,
    /// Human-readable banners, in display order.
    pub banners: Vec<String>,
}

impl StartupReport {
    pub fn from_ready(ready: Option<&Ready>) -> Self {
        let mut r = StartupReport { cache_state: "absent".into(), ..Default::default() };
        if let Some(w) = ready {
            r.worker_ready = true;
            r.worker_threads = w.threads;
            r.build_features = w.build_features.clone();
            r.cpu_features = w.cpu_features.clone();
            r.capabilities = w.capabilities.clone();
            r.cpu_lacks_avx2 = cpu_lacks_avx2(w);
            if r.cpu_lacks_avx2 { r.banners.push("this CPU does not report AVX2; solves will be much slower".into()); }
        } else {
            r.banners.push("the solver worker is not running".into());
        }
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeWorker;

    /// A link with no live worker reports it, with the cache still absent; a CPU without AVX2 is a banner only.
    #[test]
    fn no_worker_and_a_cpu_without_avx2_are_banners() {
        let none = StartupReport::from_ready(None);
        assert_eq!(none, StartupReport { cache_state: "absent".into(), banners: vec!["the solver worker is not running".into()], ..Default::default() });
        let mut ready = FakeWorker::default_ready();
        ready.cpu_features.clear();
        let r = StartupReport::from_ready(Some(&ready));
        assert!(r.worker_ready && r.cpu_lacks_avx2 && r.worker_threads == 16 && r.build_features == ready.build_features);
        assert_eq!(r.banners, vec!["this CPU does not report AVX2; solves will be much slower".to_string()]);
    }
}
