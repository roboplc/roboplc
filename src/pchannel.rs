use std::sync::Arc;

use crate::{pdeque::Deque, Error, MessageDeliveryPolicy, Result};
use object_id::UniqueId;
use parking_lot::{Condvar, Mutex};

struct Channel<T: MessageDeliveryPolicy>(Arc<ChannelInner<T>>);

impl<T: MessageDeliveryPolicy> Channel<T> {
    fn id(&self) -> usize {
        self.0.id.as_usize()
    }
}

impl<T: MessageDeliveryPolicy> Eq for Channel<T> {}

impl<T: MessageDeliveryPolicy> PartialEq for Channel<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl<T> Clone for Channel<T>
where
    T: MessageDeliveryPolicy,
{
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

struct ChannelInner<T: MessageDeliveryPolicy> {
    id: UniqueId,
    pc: Mutex<PolicyChannel<T>>,
    available: Condvar,
}

impl<T: MessageDeliveryPolicy> ChannelInner<T> {
    fn try_send(&self, value: T) -> Result<()> {
        let mut pc = self.pc.lock();
        if pc.receivers == 0 {
            return Err(Error::ChannelClosed);
        }
        let push_result = pc.queue.try_push(value);
        if push_result.value.is_none() {
            self.available.notify_one();
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
            self.available.wait(&mut pc);
        };
        self.available.notify_one();
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
                self.available.notify_one();
                return Ok(val);
            } else if pc.senders == 0 {
                return Err(Error::ChannelClosed);
            }
            self.available.wait(&mut pc);
        }
    }
    fn try_recv(&self) -> Result<T> {
        let mut pc = self.pc.lock();
        if let Some(val) = pc.queue.get() {
            self.available.notify_one();
            Ok(val)
        } else if pc.senders == 0 {
            Err(Error::ChannelClosed)
        } else {
            Err(Error::ChannelEmpty)
        }
    }
}

impl<T: MessageDeliveryPolicy> Channel<T> {
    fn new(capacity: usize, ordering: bool) -> Self {
        Self(
            ChannelInner {
                id: <_>::default(),
                pc: Mutex::new(PolicyChannel::new(capacity, ordering)),
                available: Condvar::new(),
            }
            .into(),
        )
    }
}

struct PolicyChannel<T: MessageDeliveryPolicy> {
    queue: Deque<T>,
    senders: usize,
    receivers: usize,
}

impl<T> PolicyChannel<T>
where
    T: MessageDeliveryPolicy,
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
    T: MessageDeliveryPolicy,
{
    channel: Channel<T>,
}

impl<T> Sender<T>
where
    T: MessageDeliveryPolicy,
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
    T: MessageDeliveryPolicy,
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
    T: MessageDeliveryPolicy,
{
    fn drop(&mut self) {
        self.channel.0.pc.lock().senders -= 1;
        self.channel.0.available.notify_all();
    }
}

#[derive(Eq, PartialEq)]
pub struct Receiver<T>
where
    T: MessageDeliveryPolicy,
{
    channel: Channel<T>,
}

impl<T> Receiver<T>
where
    T: MessageDeliveryPolicy,
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
    T: MessageDeliveryPolicy,
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
    T: MessageDeliveryPolicy,
{
    fn drop(&mut self) {
        self.channel.0.pc.lock().receivers -= 1;
        self.channel.0.available.notify_all();
    }
}

fn make_channel<T: MessageDeliveryPolicy>(ch: Channel<T>) -> (Sender<T>, Receiver<T>) {
    let tx = Sender {
        channel: ch.clone(),
    };
    let rx = Receiver { channel: ch };
    (tx, rx)
}

/// Creates a bounded channel which respects [`MessageDeliveryPolicy`] rules with no message
/// priority ordering
///
/// # Panics
///
/// Will panic if the capacity is zero
pub fn bounded<T: MessageDeliveryPolicy>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let ch = Channel::new(capacity, false);
    make_channel(ch)
}

/// Creates a bounded channel which respects [`MessageDeliveryPolicy`] rules and has got message
/// priority ordering turned on
///
/// # Panics
///
/// Will panic if the capacity is zero
pub fn ordered<T: MessageDeliveryPolicy>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let ch = Channel::new(capacity, true);
    make_channel(ch)
}

#[cfg(test)]
mod test {
    use std::{thread, time::Duration};

    use crate::{DeliveryPolicy, MessageDeliveryPolicy};

    use super::bounded;

    #[derive(Debug)]
    enum Message {
        Test(usize),
        Temperature(f64),
        Spam,
    }

    impl MessageDeliveryPolicy for Message {
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
                    assert!(e.is_skipped(), "{}", e);
                }
                tx.send(Message::Temperature(123.0)).unwrap();
            }
        });
        thread::sleep(Duration::from_secs(1));
        while let Ok(msg) = rx.recv() {
            thread::sleep(Duration::from_millis(10));
            if matches!(msg, Message::Spam) {
                panic!("delivery policy not respected ({:?})", msg);
            }
        }
    }

    #[test]
    fn test_delivery_policy_single() {
        let (tx, rx) = bounded::<Message>(512);
        thread::spawn(move || {
            for _ in 0..10 {
                tx.send(Message::Test(123)).unwrap();
                if let Err(e) = tx.send(Message::Spam) {
                    assert!(e.is_skipped(), "{}", e);
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
        assert_eq!(c, 10);
        assert_eq!(t, 1);
    }

    #[test]
    fn test_poisoning() {
        let n = 20_000;
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
