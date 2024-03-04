use std::collections::VecDeque;

use parking_lot::Mutex;

/// A capacity-limited thread-safe deque-based data buffer
pub struct DataBuffer<T> {
    data: Mutex<VecDeque<T>>,
    capacity: usize,
}

impl<T> DataBuffer<T> {
    /// # Panics
    ///
    /// Will panic if the capacity is zero
    #[inline]
    pub fn bounded(capacity: usize) -> Self {
        assert!(capacity > 0, "data buffer capacity MUST be > 0");
        Self {
            data: <_>::default(),
            capacity,
        }
    }
    /// Tries to push the value
    /// returns the value back if not pushed
    pub fn try_push(&self, value: T) -> Option<T> {
        let mut buf = self.data.lock();
        if buf.len() >= self.capacity {
            return Some(value);
        }
        buf.push_back(value);
        None
    }
    /// Forcibly pushes the value, removing the first element if necessary
    ///
    /// returns true in case the buffer had enough capacity or false if the first element had been
    /// removed
    pub fn force_push(&self, value: T) -> bool {
        let mut buf = self.data.lock();
        let mut res = true;
        while buf.len() >= self.capacity {
            buf.pop_front();
            res = false;
        }
        buf.push_back(value);
        res
    }
    /// the current buffer length (number of elements)
    pub fn len(&self) -> usize {
        self.data.lock().len()
    }
    /// is the buffer empty
    pub fn is_empty(&self) -> bool {
        self.data.lock().is_empty()
    }
    /// takes the buffer content and keeps nothing inside
    pub fn take(&self) -> VecDeque<T> {
        std::mem::take(&mut *self.data.lock())
    }
}
