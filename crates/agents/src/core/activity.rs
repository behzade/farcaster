use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicU64, Ordering},
    },
    thread::Thread,
};

struct Activity {
    revision: AtomicU64,
    wake: Thread,
}

fn subscribers() -> &'static Mutex<HashMap<PathBuf, Vec<Weak<Activity>>>> {
    static SUBSCRIBERS: OnceLock<Mutex<HashMap<PathBuf, Vec<Weak<Activity>>>>> = OnceLock::new();
    SUBSCRIBERS.get_or_init(Mutex::default)
}

pub struct WorkerActivitySubscription {
    project: PathBuf,
    activity: Arc<Activity>,
}

impl WorkerActivitySubscription {
    pub fn revision(&self) -> u64 {
        self.activity.revision.load(Ordering::Acquire)
    }
}

impl Drop for WorkerActivitySubscription {
    fn drop(&mut self) {
        let mut subscribers = subscribers().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(project) = subscribers.get_mut(&self.project) {
            let own = Arc::downgrade(&self.activity);
            project.retain(|subscriber| subscriber.strong_count() > 0 && !subscriber.ptr_eq(&own));
            if project.is_empty() {
                subscribers.remove(&self.project);
            }
        }
    }
}

pub fn subscribe_worker_activity(project: &Path, wake: Thread) -> WorkerActivitySubscription {
    let activity = Arc::new(Activity {
        revision: AtomicU64::new(0),
        wake,
    });
    subscribers()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(project.to_owned())
        .or_default()
        .push(Arc::downgrade(&activity));
    WorkerActivitySubscription {
        project: project.to_owned(),
        activity,
    }
}

pub(super) fn record(project: &Path) {
    let subscribers = subscribers().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(project) = subscribers.get(project) {
        for activity in project.iter().filter_map(Weak::upgrade) {
            activity.revision.fetch_add(1, Ordering::Release);
            activity.wake.unpark();
        }
    }
}

#[cfg(test)]
#[path = "activity_tests.rs"]
mod tests;
