use std::sync::Arc;

use crate::{pdeque::Deque, DataDeliveryPolicy, Error, Result};
use object_id::UniqueId;
use parking_lot::{Condvar, Mutex};

/// An abstract trait for data channels and hubs
pub trait DataChannel<T: DataDeliveryPolicy> {
    fn send(&self, value: T) -> Result<()>;
    fn try_send(&self, value: T) -> Result<()>;
    fn recv(&self) -> Result<T>;
    fn try_recv(&self) -> Result<T>;
    fn is_alive(&self) -> bool {
        true
    }
}

impl<T> DataChannel<T> for Sender<T>
where
    T: DataDeliveryPolicy,
{
    fn send(&self, value: T) -> Result<()> {
        self.send(value)
    }
    fn try_send(&self, value: T) -> Result<()> {
        self.try_send(value)
    }
    fn try_recv(&self) -> Result<T> {
        Err(Error::Unimplemented)
    }
    fn recv(&self) -> Result<T> {
        Err(Error::Unimplemented)
    }
    fn is_alive(&self) -> bool {
        self.is_alive()
    }
}

impl<T> DataChannel<T> for Receiver<T>
where
    T: DataDeliveryPolicy,
{
    fn send(&self, _value: T) -> Result<()> {
        Err(Error::Unimplemented)
    }
    fn try_send(&self, _value: T) -> Result<()> {
        Err(Error::Unimplemented)
    }
    fn try_recv(&self) -> Result<T> {
        self.try_recv()
    }
    fn recv(&self) -> Result<T> {
        self.recv()
    }
    fn is_alive(&self) -> bool {
        self.is_alive()
    }
}

struct Channel<T: DataDeliveryPolicy>(Arc<ChannelInner<T>>);

impl<T: DataDeliveryPolicy> Channel<T> {
    fn id(&self) -> usize {
        self.0.id.as_usize()
    }
}

impl<T: DataDeliveryPolicy> Eq for Channel<T> {}

impl<T: DataDeliveryPolicy> PartialEq for Channel<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl<T> Clone for Channel<T>
where
    T: DataDeliveryPolicy,
{
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

struct ChannelInner<T: DataDeliveryPolicy> {
    id: UniqueId,
    pc: Mutex<PolicyChannel<T>>,
    data_available: Condvar,
    space_available: Condvar,
}

impl<T: DataDeliveryPolicy> ChannelInner<T> {
    fn try_send(&self, value: T) -> Result<()> {
        let mut pc = self.pc.lock();
        if pc.receivers == 0 {
            return Err(Error::ChannelClosed);
        }
        let push_result = pc.queue.try_push(value);
        if push_result.value.is_none() {
            self.data_available.notify_one();
            if push_result.pushed {
                Ok(())
            } else {
                Err(Error::ChannelSkipped)
            }
        } else {
            Err(Error::ChannelFull)
        }
    }
    fn send(&self, mut value: T) -> Result<()> {
        let mut pc = self.pc.lock();
        let pushed = loop {
            if pc.receivers == 0 {
                return Err(Error::ChannelClosed);
            }
            let push_result = pc.queue.try_push(value);
            let Some(val) = push_result.value else {
                break push_result.pushed;
            };
            value = val;
            self.space_available.wait(&mut pc);
        };
        self.data_available.notify_one();
        if pushed {
            Ok(())
        } else {
            Err(Error::ChannelSkipped)
        }
    }
    fn recv(&self) -> Result<T> {
        let mut pc = self.pc.lock();
        loop {
            if let Some(val) = pc.queue.get() {
                self.space_available.notify_one();
                return Ok(val);
            } else if pc.senders == 0 {
                return Err(Error::ChannelClosed);
            }
            self.data_available.wait(&mut pc);
        }
    }
    fn try_recv(&self) -> Result<T> {
        let mut pc = self.pc.lock();
        if let Some(val) = pc.queue.get() {
            self.space_available.notify_one();
            Ok(val)
        } else if pc.senders == 0 {
            Err(Error::ChannelClosed)
        } else {
            Err(Error::ChannelEmpty)
        }
    }
}

impl<T: DataDeliveryPolicy> Channel<T> {
    fn new(capacity: usize, ordering: bool) -> Self {
        Self(
            ChannelInner {
                id: <_>::default(),
                pc: Mutex::new(PolicyChannel::new(capacity, ordering)),
                data_available: Condvar::new(),
                space_available: Condvar::new(),
            }
            .into(),
        )
    }
}

struct PolicyChannel<T: DataDeliveryPolicy> {
    queue: Deque<T>,
    senders: usize,
    receivers: usize,
}

impl<T> PolicyChannel<T>
where
    T: DataDeliveryPolicy,
{
    fn new(capacity: usize, ordering: bool) -> Self {
        assert!(capacity > 0, "channel capacity MUST be > 0");
        Self {
            queue: Deque::bounded(capacity).set_ordering(ordering),
            senders: 1,
            receivers: 1,
        }
    }
}

#[derive(Eq, PartialEq)]
pub struct Sender<T>
where
    T: DataDeliveryPolicy,
{
    channel: Channel<T>,
}

impl<T> Sender<T>
where
    T: DataDeliveryPolicy,
{
    #[inline]
    pub fn send(&self, value: T) -> Result<()> {
        self.channel.0.send(value)
    }
    #[inline]
    pub fn try_send(&self, value: T) -> Result<()> {
        self.channel.0.try_send(value)
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.channel.0.pc.lock().queue.len()
    }
    #[inline]
    pub fn is_full(&self) -> bool {
        self.channel.0.pc.lock().queue.is_full()
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.channel.0.pc.lock().queue.is_empty()
    }
    #[inline]
    pub fn is_alive(&self) -> bool {
        self.channel.0.pc.lock().receivers > 0
    }
}

impl<T> Clone for Sender<T>
where
    T: DataDeliveryPolicy,
{
    fn clone(&self) -> Self {
        self.channel.0.pc.lock().senders += 1;
        Self {
            channel: self.channel.clone(),
        }
    }
}

impl<T> Drop for Sender<T>
where
    T: DataDeliveryPolicy,
{
    fn drop(&mut self) {
        let mut pc = self.channel.0.pc.lock();
        pc.senders -= 1;
        if pc.senders == 0 {
            self.channel.0.data_available.notify_all();
        }
    }
}

#[derive(Eq, PartialEq)]
pub struct Receiver<T>
where
    T: DataDeliveryPolicy,
{
    channel: Channel<T>,
}

impl<T> Iterator for Receiver<T>
where
    T: DataDeliveryPolicy,
{
    type Item = T;
    fn next(&mut self) -> Option<Self::Item> {
        self.recv().ok()
    }
}

impl<T> Receiver<T>
where
    T: DataDeliveryPolicy,
{
    #[inline]
    pub fn recv(&self) -> Result<T> {
        self.channel.0.recv()
    }
    #[inline]
    pub fn try_recv(&self) -> Result<T> {
        self.channel.0.try_recv()
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.channel.0.pc.lock().queue.len()
    }
    #[inline]
    pub fn is_full(&self) -> bool {
        self.channel.0.pc.lock().queue.is_full()
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.channel.0.pc.lock().queue.is_empty()
    }
    #[inline]
    pub fn is_alive(&self) -> bool {
        self.channel.0.pc.lock().senders > 0
    }
}

impl<T> Clone for Receiver<T>
where
    T: DataDeliveryPolicy,
{
    fn clone(&self) -> Self {
        self.channel.0.pc.lock().receivers += 1;
        Self {
            channel: self.channel.clone(),
        }
    }
}

impl<T> Drop for Receiver<T>
where
    T: DataDeliveryPolicy,
{
    fn drop(&mut self) {
        let mut pc = self.channel.0.pc.lock();
        pc.receivers -= 1;
        if pc.receivers == 0 {
            self.channel.0.data_available.notify_all();
        }
    }
}

fn make_channel<T: DataDeliveryPolicy>(ch: Channel<T>) -> (Sender<T>, Receiver<T>) {
    let tx = Sender {
        channel: ch.clone(),
    };
    let rx = Receiver { channel: ch };
    (tx, rx)
}

/// Creates a bounded channel which respects [`DataDeliveryPolicy`] rules with no message
/// priority ordering
///
/// # Panics
///
/// Will panic if the capacity is zero
pub fn bounded<T: DataDeliveryPolicy>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let ch = Channel::new(capacity, false);
    make_channel(ch)
}

/// Creates a bounded channel which respects [`DataDeliveryPolicy`] rules and has got message
/// priority ordering turned on
///
/// # Panics
///
/// Will panic if the capacity is zero
pub fn ordered<T: DataDeliveryPolicy>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let ch = Channel::new(capacity, true);
    make_channel(ch)
}

#[cfg(test)]
mod test {
    use std::{thread, time::Duration};

    use crate::{DataDeliveryPolicy, DeliveryPolicy};

    use super::bounded;

    #[derive(Debug)]
    enum Message {
        Test(usize),
        Temperature(f64),
        Spam,
    }

    impl DataDeliveryPolicy for Message {
        fn delivery_policy(&self) -> DeliveryPolicy {
            match self {
                Message::Test(_) => DeliveryPolicy::Always,
                Message::Temperature(_) => DeliveryPolicy::Single,
                Message::Spam => DeliveryPolicy::Optional,
            }
        }
    }

    #[test]
    fn test_delivery_policy_optional() {
        let (tx, rx) = bounded::<Message>(1);
        thread::spawn(move || {
            for _ in 0..10 {
                tx.send(Message::Test(123)).unwrap();
                if let Err(e) = tx.send(Message::Spam) {
                    assert!(e.is_data_skipped(), "{}", e);
                }
                tx.send(Message::Temperature(123.0)).unwrap();
            }
        });
        thread::sleep(Duration::from_secs(1));
        let mut messages = Vec::new();
        while let Ok(msg) = rx.recv() {
            thread::sleep(Duration::from_millis(10));
            if matches!(msg, Message::Spam) {
                panic!("delivery policy not respected ({:?})", msg);
            }
            messages.push(msg);
        }
        insta::assert_debug_snapshot!(messages.len(), @"20");
    }

    #[test]
    fn test_delivery_policy_single() {
        let (tx, rx) = bounded::<Message>(512);
        thread::spawn(move || {
            for _ in 0..10 {
                tx.send(Message::Test(123)).unwrap();
                if let Err(e) = tx.send(Message::Spam) {
                    assert!(e.is_data_skipped(), "{}", e);
                }
                tx.send(Message::Temperature(123.0)).unwrap();
            }
        });
        thread::sleep(Duration::from_secs(1));
        let mut c = 0;
        let mut t = 0;
        while let Ok(msg) = rx.recv() {
            match msg {
                Message::Test(_) => c += 1,
                Message::Temperature(_) => t += 1,
                Message::Spam => {}
            }
        }
        insta::assert_snapshot!(c, @"10");
        insta::assert_snapshot!(t, @"1");
    }

    #[test]
    fn test_poisoning() {
        let n = 5_000;
        for i in 0..n {
            let (tx, rx) = bounded::<Message>(512);
            let rx_t = thread::spawn(move || while rx.recv().is_ok() {});
            thread::spawn(move || {
                let _t = tx;
            });
            for _ in 0..100 {
                if rx_t.is_finished() {
                    break;
                }
                thread::sleep(Duration::from_millis(1));
            }
            assert!(rx_t.is_finished(), "RX poisined {}", i);
        }
    }
}
