use bma_ts::Monotonic;
use std::{ops::Deref, time::Duration};

/// A memory cell with an expiring value with API similar to the standard [`Option`]
pub struct TtlCell<T> {
    value: Option<T>,
    ttl: Duration,
    set_at: Monotonic,
}

impl<T> TtlCell<T> {
    /// Creates a new empty cell
    #[inline]
    pub fn new(ttl: Duration) -> Self {
        Self {
            value: None,
            ttl,
            set_at: Monotonic::now(),
        }
    }
    /// Creates a new empty cell with a value set
    #[inline]
    pub fn new_with_value(ttl: Duration, value: T) -> Self {
        Self {
            value: Some(value),
            ttl,
            set_at: Monotonic::now(),
        }
    }
    /// Replaces the current value, returns the previous one. The value set time is set to the
    /// current time point
    #[inline]
    pub fn replace(&mut self, value: T) -> Option<T> {
        let prev = self.value.replace(value);
        let result = if self.is_expired() { None } else { prev };
        self.touch();
        result
    }
    /// Sets the current value. The value set time is set to the current time point
    #[inline]
    pub fn set(&mut self, value: T) {
        self.value = Some(value);
        self.touch();
    }
    /// Clears the current value
    #[inline]
    pub fn clear(&mut self) {
        self.value = None;
    }
    /// Returns a refernce to the value if set and not expired
    #[inline]
    pub fn as_ref(&self) -> Option<&T> {
        if self.is_expired() {
            None
        } else {
            self.value.as_ref()
        }
    }
    /// A value ref-coupler
    ///
    /// Returns two references to two [`TtlCell`] values in case if both of them are not expired and
    /// set time difference matches "max_time_delta" parameter
    #[inline]
    pub fn as_ref_with<'a, O>(
        &'a self,
        other: &'a TtlCell<O>,
        max_time_delta: Duration,
    ) -> Option<(&T, &O)> {
        let maybe_first = self.as_ref();
        let maybe_second = other.as_ref();
        if let Some(first) = maybe_first {
            if let Some(second) = maybe_second {
                if self.set_at.abs_diff(other.set_at) <= max_time_delta {
                    return Some((first, second));
                }
            }
        }
        None
    }
    /// Takes the value if set and not expired, clears the cell
    #[inline]
    pub fn take(&mut self) -> Option<T> {
        if self.is_expired() {
            None
        } else {
            self.value.take()
        }
    }
    /// A value take-coupler
    ///
    /// Takes two [`TtlCell`] values in case if both of them are not expired and set time
    /// difference matches "max_time_delta" parameter. Both cells are cleared.
    #[inline]
    pub fn take_with<O>(
        &mut self,
        other: &mut TtlCell<O>,
        max_time_delta: Duration,
    ) -> Option<(T, O)> {
        let maybe_first = self.take();
        let maybe_second = other.take();
        if let Some(first) = maybe_first {
            if let Some(second) = maybe_second {
                if self.set_at.abs_diff(other.set_at) <= max_time_delta {
                    return Some((first, second));
                }
            }
        }
        None
    }
    /// Returns a derefernce to the value if set and not expired
    #[inline]
    pub fn as_deref(&self) -> Option<&T::Target>
    where
        T: Deref,
    {
        match self.as_ref() {
            Some(t) => Some(&**t),
            None => None,
        }
    }
    /// Returns true if the value is expired or not set
    #[inline]
    pub fn is_expired(&self) -> bool {
        self.set_at.elapsed() > self.ttl || self.value.is_none()
    }
    /// Updates the value set time to the current point of time
    #[inline]
    pub fn touch(&mut self) {
        self.set_at = Monotonic::now();
    }
    /// Returns the value set time (monotonic)
    #[inline]
    pub fn set_at(&self) -> Monotonic {
        self.set_at
    }
}

#[cfg(test)]
mod test {
    use std::{thread, time::Duration};

    use super::TtlCell;

    #[test]
    fn test_get_set() {
        let ttl = Duration::from_millis(10);
        let mut opt = TtlCell::new_with_value(ttl, 25);
        thread::sleep(ttl / 2);
        insta::assert_debug_snapshot!(opt.as_ref().copied(), @r###"
        Some(
            25,
        )
        "###);
        thread::sleep(ttl);
        insta::assert_debug_snapshot!(opt.as_ref().copied(), @"None");
        opt.set(30);
        thread::sleep(ttl / 2);
        insta::assert_debug_snapshot!(opt.as_ref().copied(), @r###"
        Some(
            30,
        )
        "###);
        thread::sleep(ttl);
        insta::assert_debug_snapshot!(opt.as_ref().copied(), @"None");
    }
    #[test]
    fn test_take_replace() {
        let ttl = Duration::from_millis(10);
        let mut opt = TtlCell::new_with_value(ttl, 25);
        thread::sleep(ttl / 2);
        insta::assert_debug_snapshot!(opt.take(), @r###"
        Some(
            25,
        )
        "###);
        insta::assert_debug_snapshot!(opt.as_ref().copied(), @"None");
        opt.set(30);
        thread::sleep(ttl / 2);
        insta::assert_debug_snapshot!(opt.replace(29), @r###"
        Some(
            30,
        )
        "###);
        thread::sleep(ttl);
        insta::assert_debug_snapshot!(opt.as_ref().copied(), @"None");
    }
    #[test]
    fn test_take_with() {
        let mut first = TtlCell::new_with_value(Duration::from_secs(1), 25);
        thread::sleep(Duration::from_millis(10));
        let mut second = TtlCell::new_with_value(Duration::from_secs(1), 25);
        insta::assert_debug_snapshot!(first
            .take_with(&mut second, Duration::from_millis(100)), @r###"
        Some(
            (
                25,
                25,
            ),
        )
        "###);
        let mut first = TtlCell::new_with_value(Duration::from_secs(1), 25);
        thread::sleep(Duration::from_millis(100));
        let mut second = TtlCell::new_with_value(Duration::from_secs(1), 25);
        insta::assert_debug_snapshot!(
            first.take_with(&mut second, Duration::from_millis(50)), @"None");
    }
}
