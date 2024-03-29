use std::sync::Arc;

use parking_lot::{Condvar, Mutex};

/// A lightweight real-time safe semaphore
pub struct Semaphore {
    inner: Arc<SemaphoreInner>,
}

impl Semaphore {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: SemaphoreInner {
                permissions: <_>::default(),
                capacity,
                cv: Condvar::new(),
            }
            .into(),
        }
    }
    /// Tries to acquire permission, returns None if failed
    pub fn try_acquire(&self) -> Option<SemaphoreGuard> {
        let mut count = self.inner.permissions.lock();
        if *count == self.inner.capacity {
            return None;
        }
        *count += 1;
        Some(SemaphoreGuard {
            inner: self.inner.clone(),
        })
    }
    /// Acquires permission, blocks until it is available
    pub fn acquire(&self) -> SemaphoreGuard {
        let mut count = self.inner.permissions.lock();
        while *count == self.inner.capacity {
            self.inner.cv.wait(&mut count);
        }
        *count += 1;
        SemaphoreGuard {
            inner: self.inner.clone(),
        }
    }
    pub fn capacity(&self) -> usize {
        self.inner.capacity
    }
    pub fn available(&self) -> usize {
        self.inner.capacity - *self.inner.permissions.lock()
    }
    pub fn used(&self) -> usize {
        *self.inner.permissions.lock()
    }
    /// For tests only
    #[allow(dead_code)]
    fn is_poisoned(&self) -> bool {
        *self.inner.permissions.lock() > self.inner.capacity
    }
}

struct SemaphoreInner {
    permissions: Mutex<usize>,
    capacity: usize,
    cv: Condvar,
}

impl SemaphoreInner {
    fn release(&self) {
        let mut count = self.permissions.lock();
        *count -= 1;
        self.cv.notify_one();
    }
}

#[allow(clippy::module_name_repetitions)]
pub struct SemaphoreGuard {
    inner: Arc<SemaphoreInner>,
}

impl Drop for SemaphoreGuard {
    fn drop(&mut self) {
        self.inner.release();
    }
}

#[cfg(test)]
mod test {
    use std::time::Instant;

    use super::*;

    #[test]
    fn test_semaphore() {
        let sem = Semaphore::new(2);
        assert_eq!(sem.capacity(), 2);
        assert_eq!(sem.available(), 2);
        assert_eq!(sem.used(), 0);
        let _g1 = sem.acquire();
        assert_eq!(sem.available(), 1);
        assert_eq!(sem.used(), 1);
        let _g2 = sem.acquire();
        assert_eq!(sem.available(), 0);
        assert_eq!(sem.used(), 2);
        let g3 = sem.try_acquire();
        assert!(g3.is_none());
        drop(_g1);
        assert_eq!(sem.available(), 1);
        assert_eq!(sem.used(), 1);
        let _g4 = sem.acquire();
        assert_eq!(sem.available(), 0);
        assert_eq!(sem.used(), 2);
    }
    #[test]
    fn test_semaphore_multithread() {
        let start = Instant::now();
        let sem = Semaphore::new(10);
        let mut tasks = Vec::new();
        for _ in 0..100 {
            let perm = sem.acquire();
            tasks.push(std::thread::spawn(move || {
                let _perm = perm;
                std::thread::sleep(std::time::Duration::from_millis(1));
            }));
        }
        'outer: loop {
            for task in &tasks {
                std::hint::spin_loop();
                assert!(!sem.is_poisoned(), "Semaphore is poisoned");
                if !task.is_finished() {
                    continue 'outer;
                }
            }
            break 'outer;
        }
        assert!(start.elapsed().as_millis() > 10);
    }
}
