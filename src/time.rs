use std::{thread, time::Duration};

use bma_ts::Monotonic;

/// A trait which extends the standard [`Duration`] and similar types with additional methods
///
pub trait DurationRT {
    /// Returns true if all provided [`Monotonic`] times fit the duration
    fn fits(&self, t: &[Monotonic]) -> bool;
}

impl DurationRT for Duration {
    fn fits(&self, t: &[Monotonic]) -> bool {
        if t.is_empty() {
            true
        } else {
            let min_ts = t.iter().min().unwrap();
            let max_ts = t.iter().max().unwrap();
            max_ts.as_duration() - min_ts.as_duration() <= *self
        }
    }
}

/// A synchronous interval helper, similar to
/// <https://docs.rs/tokio/latest/tokio/time/struct.Interval.html>
pub struct Interval {
    next_tick: Option<Monotonic>,
    period: Duration,
    missing_tick_behavior: MissedTickBehavior,
}

impl Interval {
    pub fn new(period: Duration) -> Self {
        Self {
            next_tick: None,
            period,
            missing_tick_behavior: <_>::default(),
        }
    }
    /// Ticks the interval
    ///
    /// Returns false if a tick is missed
    pub fn tick(&mut self) -> bool {
        let now = Monotonic::now();
        if let Some(mut next_tick) = self.next_tick {
            match now.cmp(&next_tick) {
                std::cmp::Ordering::Less => {
                    let to_sleep = next_tick - now;
                    self.next_tick = Some(next_tick + self.period);
                    thread::sleep(to_sleep);
                    true
                }
                std::cmp::Ordering::Equal => true,
                std::cmp::Ordering::Greater => {
                    match self.missing_tick_behavior {
                        MissedTickBehavior::Burst => {
                            self.next_tick = Some(next_tick + self.period);
                        }
                        MissedTickBehavior::Delay => {
                            self.next_tick = Some(now + self.period);
                        }
                        MissedTickBehavior::Skip => {
                            while next_tick <= now {
                                next_tick += self.period;
                            }
                            self.next_tick = Some(next_tick);
                        }
                    }
                    false
                }
            }
        } else {
            self.next_tick = Some(now + self.period);
            true
        }
    }
    /// Sets missing tick behavior policy. Can be used as a build pattern
    pub fn set_missing_tick_behavior(mut self, missing_tick_behavior: MissedTickBehavior) -> Self {
        self.missing_tick_behavior = missing_tick_behavior;
        self
    }
}

/// Interval missing tick behavior
///
/// The behavior is similar to
/// <https://docs.rs/tokio/latest/tokio/time/enum.MissedTickBehavior.html>
/// but may differ in some details
#[derive(Debug, Copy, Clone, Eq, PartialEq, Default)]
pub enum MissedTickBehavior {
    #[default]
    /// `[Interval::tick()`] method has no delay for missed intervals, all the missed ones are
    /// fired instantly
    Burst,
    /// The interval is restarted from the current point of time
    Delay,
    /// Missed ticks are skipped with no additional effect
    Skip,
}

#[cfg(test)]
mod test {
    use std::{thread, time::Duration};

    use bma_ts::Monotonic;

    use crate::time::DurationRT as _;

    #[test]
    fn test_fits() {
        let first = Monotonic::now();
        thread::sleep(Duration::from_millis(10));
        let second = Monotonic::now();
        thread::sleep(Duration::from_millis(10));
        let third = Monotonic::now();
        assert!(Duration::from_millis(100).fits(&[first, second, third]));
        assert!(Duration::from_millis(25).fits(&[first, second, third]));
    }
}
