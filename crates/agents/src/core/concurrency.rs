use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug)]
pub struct WorkerConcurrency {
    maximum: usize,
    active: Arc<Mutex<usize>>,
    profiles: Arc<Mutex<BTreeMap<String, (usize, usize)>>>,
}

impl WorkerConcurrency {
    pub fn new(maximum: usize) -> Self {
        Self {
            maximum,
            active: Arc::new(Mutex::new(0)),
            profiles: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn reserve(&self) -> Result<WorkerSlot, String> {
        let slot = WorkerSlot(Arc::new(SlotState {
            concurrency: self.clone(),
            active: Mutex::new(false),
            profile: None,
        }));
        if !slot.try_activate() {
            return Err(format!(
                "worker pool is full (maximum {} active workers)",
                self.maximum
            ));
        }
        Ok(slot)
    }

    pub fn set_profile_limits(
        &self,
        limits: impl IntoIterator<Item = (String, usize)>,
    ) -> Result<(), String> {
        let mut profiles = self
            .profiles
            .lock()
            .map_err(|_| "worker profile slots are unavailable")?;
        for (name, limit) in limits {
            let active = profiles.get(&name).map_or(0, |(_, active)| *active);
            profiles.insert(name, (limit, active));
        }
        Ok(())
    }

    pub fn reserve_profile(&self, name: &str) -> Result<WorkerSlot, String> {
        let slot = WorkerSlot(Arc::new(SlotState {
            concurrency: self.clone(),
            active: Mutex::new(false),
            profile: Some(name.into()),
        }));
        if !slot.try_activate() {
            return Err(format!("worker profile '{name}' is full"));
        }
        Ok(slot)
    }
}

#[derive(Clone, Debug)]
pub struct WorkerSlot(Arc<SlotState>);

#[derive(Debug)]
struct SlotState {
    concurrency: WorkerConcurrency,
    active: Mutex<bool>,
    profile: Option<String>,
}

impl WorkerSlot {
    pub fn is_active(&self) -> bool {
        self.0.active.lock().is_ok_and(|active| *active)
    }

    pub fn try_activate(&self) -> bool {
        let Ok(mut active) = self.0.active.lock() else {
            return false;
        };
        if *active {
            return true;
        }
        if let Some(profile) = &self.0.profile {
            let Ok(mut profiles) = self.0.concurrency.profiles.lock() else {
                return false;
            };
            let (limit, count) = profiles.entry(profile.clone()).or_insert((10, 0));
            if *count >= *limit {
                return false;
            }
            *count += 1;
        } else {
            let Ok(mut count) = self.0.concurrency.active.lock() else {
                return false;
            };
            if *count >= self.0.concurrency.maximum {
                return false;
            }
            *count += 1;
        }
        *active = true;
        true
    }

    pub fn release(&self) {
        if let Ok(mut active) = self.0.active.lock()
            && *active
        {
            self.0.release_count();
            *active = false;
        }
    }
}

impl Drop for SlotState {
    fn drop(&mut self) {
        if self.active.get_mut().is_ok_and(|active| *active) {
            self.release_count();
        }
    }
}

impl SlotState {
    fn release_count(&self) {
        if let Some(profile) = &self.profile {
            if let Ok(mut profiles) = self.concurrency.profiles.lock()
                && let Some((_, count)) = profiles.get_mut(profile)
            {
                *count -= 1;
            }
        } else if let Ok(mut count) = self.concurrency.active.lock() {
            *count -= 1;
        }
    }
}

#[cfg(test)]
#[path = "concurrency_tests.rs"]
mod tests;
