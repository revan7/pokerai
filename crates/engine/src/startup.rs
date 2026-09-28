//! §12 startup diagnostics, rendered by plan 5's settings panel. Plan 3 fills `quarantined_bundles`,
//! plan 4 fills `cache_state`; both go through `Engine::startup_report`, never through a second channel.
use crate::worker::link::WorkerLink;
use crate::worker::ready::{cpu_lacks_avx2, ReadyRefusal};
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
    /// Spec 12 (ruling 29-I4): why the worker's `ready` was refused at startup, when it was. The engine is then degraded:
    /// `worker_ready` is false and every decision is answered with the version mismatch until the worker is rebuilt.
    pub worker_refusal: Option<ReadyRefusal>,
    /// Preflop bundles that failed validation and were quarantined (§8.2); filled by plan 3.
    pub quarantined_bundles: Vec<String>,
    /// "absent" until plan 4 opens the cache, then its own summary.
    pub cache_state: String,
    /// Human-readable banners, in display order.
    pub banners: Vec<String>,
}

impl StartupReport {
    /// The report of the engine's worker link: a degraded engine's refusal (`WorkerLink::refused`, spec 12, ruling
    /// 29-I4) with its banner, else the worker's `ready` (`from_ready`).
    pub fn from_worker(link: &dyn WorkerLink) -> Self {
        match link.refused() {
            Some(refusal) => StartupReport {
                worker_refusal: Some(refusal.clone()),
                cache_state: "absent".into(),
                banners: vec![format!("the solver worker was refused at startup (worker/proto version mismatch: {refusal}); every recommendation answers \
                    this error until the worker is rebuilt")],
                ..Default::default()
            },
            None => Self::from_ready(link.ready()),
        }
    }

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

    /// Ruling 29-I4: a link that can launch a worker reports its `ready`; a degraded engine's link reports its refusal,
    /// typed, with one banner naming the §12 mismatch.
    #[test]
    fn a_refused_worker_is_reported_with_its_typed_reason() {
        use crate::testing::{FakeClock, FakeWorker};
        use crate::worker::link::RefusedWorker;
        let (fake, _) = FakeWorker::scripted(FakeClock::new(), Default::default(), vec![]);
        assert_eq!(StartupReport::from_worker(fake.as_ref()), StartupReport::from_ready(Some(&FakeWorker::default_ready())));
        let refusal = ReadyRefusal::AdapterVersion { reported: 99 };
        let r = StartupReport::from_worker(&RefusedWorker::new("w.exe".into(), refusal.clone()));
        assert_eq!((r.worker_ready, r.worker_refusal.as_ref(), r.worker_threads, r.cache_state.as_str()), (false, Some(&refusal), 0, "absent"));
        assert_eq!(r.banners, vec![format!("the solver worker was refused at startup (worker/proto version mismatch: adapter_version 99 != {}); \
            every recommendation answers this error until the worker is rebuilt", proto::worker::ADAPTER_VERSION)]);
    }
}
