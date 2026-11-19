//! Export job ownership. Dropping a job removes its cancellation token.
use std::{collections::HashMap, sync::{Arc, Mutex, OnceLock, atomic::{AtomicBool, Ordering}}};

fn jobs() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static JOBS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    JOBS.get_or_init(Default::default)
}

pub struct Job { id: String, pub cancelled: Arc<AtomicBool> }

impl Job {
    pub fn start(id: String) -> Result<Self, String> {
        if id.is_empty() || id.len() > 80 { return Err("Invalid export identifier".into()); }
        let mut jobs = jobs().lock().unwrap_or_else(|e| e.into_inner());
        if jobs.contains_key(&id) { return Err("That export is already running".into()); }
        if jobs.len() >= 2 { return Err("Two exports are already running. Wait for one to finish.".into()); }
        let cancelled = Arc::new(AtomicBool::new(false));
        jobs.insert(id.clone(), cancelled.clone());
        Ok(Self { id, cancelled })
    }
}

impl Drop for Job {
    fn drop(&mut self) { jobs().lock().unwrap_or_else(|e| e.into_inner()).remove(&self.id); }
}

pub fn cancel(id: &str) {
    if let Some(token) = jobs().lock().unwrap_or_else(|e| e.into_inner()).get(id) {
        token.store(true, Ordering::Relaxed);
    }
}

pub fn active() -> bool { !jobs().lock().unwrap_or_else(|e| e.into_inner()).is_empty() }
