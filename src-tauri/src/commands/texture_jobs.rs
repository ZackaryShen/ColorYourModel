use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const STALE: &str = "The texture preview was cancelled or replaced";

enum Phase<T> {
    Loading,
    Ready(T),
    Applying,
}

struct TextureJob<T> {
    cancelled: Arc<AtomicBool>,
    phase: Phase<T>,
}

pub struct TextureJobs<T> {
    latest_id: u64,
    job: Option<TextureJob<T>>,
}

impl<T> Default for TextureJobs<T> {
    fn default() -> Self {
        Self {
            latest_id: 0,
            job: None,
        }
    }
}

impl<T> TextureJobs<T> {
    pub fn start(&mut self, job_id: u64) -> Result<Arc<AtomicBool>, &'static str> {
        if job_id <= self.latest_id {
            return Err(STALE);
        }
        self.invalidate();
        self.latest_id = job_id;
        let cancelled = Arc::new(AtomicBool::new(false));
        self.job = Some(TextureJob {
            cancelled: Arc::clone(&cancelled),
            phase: Phase::Loading,
        });
        Ok(cancelled)
    }

    fn current(&mut self, job_id: u64) -> Result<&mut TextureJob<T>, &'static str> {
        if job_id != self.latest_id {
            return Err(STALE);
        }
        self.job.as_mut().ok_or(STALE)
    }

    pub fn publish(&mut self, job_id: u64, candidate: T) -> Result<(), &'static str> {
        let job = self.current(job_id)?;
        if !matches!(job.phase, Phase::Loading) {
            return Err(STALE);
        }
        job.phase = Phase::Ready(candidate);
        Ok(())
    }

    pub fn take_for_apply(&mut self, job_id: u64) -> Result<T, &'static str> {
        let job = self.current(job_id)?;
        if !matches!(job.phase, Phase::Ready(_)) {
            return Err("The texture preview is not ready");
        }
        match std::mem::replace(&mut job.phase, Phase::Applying) {
            Phase::Ready(candidate) => Ok(candidate),
            _ => unreachable!("validated a ready candidate"),
        }
    }

    pub fn complete_apply(&mut self, job_id: u64) -> Result<(), &'static str> {
        if !matches!(self.current(job_id)?.phase, Phase::Applying) {
            return Err(STALE);
        }
        self.job = None;
        Ok(())
    }

    pub fn cancel(&mut self, job_id: u64) {
        if job_id >= self.latest_id {
            self.latest_id = job_id;
            self.invalidate();
        }
    }

    pub fn invalidate(&mut self) {
        if let Some(job) = self.job.take() {
            job.cancelled.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TextureJobs;
    use std::sync::atomic::Ordering;

    #[test]
    fn superseded_worker_cannot_publish_or_replace_the_new_candidate() {
        let mut jobs = TextureJobs::default();
        let old = jobs.start(1).unwrap();
        let new = jobs.start(2).unwrap();
        assert!(old.load(Ordering::Relaxed));
        assert!(!new.load(Ordering::Relaxed));
        assert!(jobs.publish(1, "old").is_err());
        jobs.publish(2, "new").unwrap();
        assert_eq!(jobs.take_for_apply(2).unwrap(), "new");
        assert!(jobs.take_for_apply(2).is_err());
        jobs.complete_apply(2).unwrap();
    }

    #[test]
    fn cancellation_arriving_before_preview_start_is_remembered() {
        let mut jobs = TextureJobs::<()>::default();
        jobs.cancel(7);
        assert!(jobs.start(7).is_err());
        assert!(jobs.start(6).is_err());
        assert!(jobs.start(8).is_ok());
    }

    #[test]
    fn cancelling_a_taken_candidate_blocks_commit_but_stale_cancel_is_ignored() {
        let mut jobs = TextureJobs::default();
        let signal = jobs.start(4).unwrap();
        jobs.cancel(3);
        assert!(!signal.load(Ordering::Relaxed));
        jobs.publish(4, 42).unwrap();
        assert_eq!(jobs.take_for_apply(4).unwrap(), 42);
        jobs.cancel(4);
        assert!(signal.load(Ordering::Relaxed));
        assert!(jobs.complete_apply(4).is_err());
    }

    #[test]
    fn replacing_the_active_model_invalidates_pending_texture_work() {
        let mut jobs = TextureJobs::default();
        jobs.start(1).unwrap();
        jobs.publish(1, 42).unwrap();
        jobs.take_for_apply(1).unwrap();
        jobs.invalidate();
        assert!(jobs.complete_apply(1).is_err());
        assert!(jobs.start(2).is_ok());
    }
}
