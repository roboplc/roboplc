use std::{
    collections::{BTreeSet, VecDeque},
    future::Future,
    mem,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll, Waker},
};

use crate::{pdeque::Deque, DataDeliveryPolicy, Error, Result};
use object_id::UniqueId;
use parking_lot::{Condvar, Mutex};
use pin_project::{pin_project, pinned_drop};

type ClientId = usize;

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
    next_op_id: AtomicUsize,
    space_available: Arc<Condvar>,
    data_available: Arc<Condvar>,
}

impl<T: DataDeliveryPolicy> Channel<T> {
    fn new(capacity: usize, ordering: bool) -> Self {
        let pc = PolicyChannel::new(capacity, ordering);
        let space_available = pc.space_available.clone();
        let data_available = pc.data_available.clone();
        Self(
            ChannelInner {
                id: <_>::default(),
                pc: Mutex::new(pc),
                next_op_id: <_>::default(),
                space_available,
                data_available,
            }
            .into(),
        )
    }
    fn op_id(&self) -> usize {
        self.0.next_op_id.fetch_add(1, Ordering::SeqCst)
    }
}

struct PolicyChannel<T: DataDeliveryPolicy> {
    queue: Deque<T>,
    senders: usize,
    receivers: usize,
    send_fut_wakers: VecDeque<Option<(Waker, ClientId)>>,
    send_fut_pending: BTreeSet<ClientId>,
    recv_fut_wakers: VecDeque<Option<(Waker, ClientId)>>,
    recv_fut_pending: BTreeSet<ClientId>,
    data_available: Arc<Condvar>,
    space_available: Arc<Condvar>,
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
            send_fut_wakers: <_>::default(),
            send_fut_pending: <_>::default(),
            recv_fut_wakers: <_>::default(),
            recv_fut_pending: <_>::default(),
            data_available: <_>::default(),
            space_available: <_>::default(),
        }
    }

    // senders

    #[inline]
    fn notify_data_sent(&mut self) {
        self.wake_next_recv();
    }

    #[inline]
    fn wake_next_send(&mut self) {
        if let Some(w) = self.send_fut_wakers.pop_front() {
            if let Some((waker, id)) = w {
                self.send_fut_pending.insert(id);
                waker.wake();
            } else {
                self.space_available.notify_one();
            }
        }
    }
    #[inline]
    fn wake_all_sends(&mut self) {
        for (waker, _) in mem::take(&mut self.send_fut_wakers).into_iter().flatten() {
            waker.wake();
        }
        self.space_available.notify_all();
    }

    #[inline]
    fn notify_send_fut_drop(&mut self, id: ClientId) {
        if let Some(pos) = self
            .send_fut_wakers
            .iter()
            .position(|w| w.as_ref().map_or(false, |(_, i)| *i == id))
        {
            self.send_fut_wakers.remove(pos);
        }
        if self.send_fut_pending.remove(&id) {
            self.wake_next_send();
        }
    }

    #[inline]
    fn confirm_send_fut_waked(&mut self, id: ClientId) {
        self.send_fut_pending.remove(&id);
    }

    #[inline]
    fn append_send_fut_waker(&mut self, waker: Waker, id: ClientId) {
        self.send_fut_wakers.push_back(Some((waker, id)));
    }

    #[inline]
    fn append_send_sync_waker(&mut self) {
        // use condvar
        self.send_fut_wakers.push_back(None);
    }

    // receivers

    #[inline]
    fn notify_data_received(&mut self) {
        self.wake_next_send();
    }

    #[inline]
    fn wake_next_recv(&mut self) {
        if let Some(w) = self.recv_fut_wakers.pop_front() {
            if let Some((waker, id)) = w {
                self.recv_fut_pending.insert(id);
                waker.wake();
            } else {
                self.data_available.notify_one();
            }
        }
    }
    #[inline]
    fn wake_all_recvs(&mut self) {
        for (waker, _) in mem::take(&mut self.recv_fut_wakers).into_iter().flatten() {
            waker.wake();
        }
        self.data_available.notify_all();
    }

    #[inline]
    fn notify_recv_fut_drop(&mut self, id: ClientId) {
        if let Some(pos) = self
            .recv_fut_wakers
            .iter()
            .position(|w| w.as_ref().map_or(false, |(_, i)| *i == id))
        {
            self.recv_fut_wakers.remove(pos);
        }
        if self.recv_fut_pending.remove(&id) {
            self.wake_next_recv();
        }
    }

    #[inline]
    fn confirm_recv_fut_waked(&mut self, id: ClientId) {
        // the resource is taken, remove from pending
        self.recv_fut_pending.remove(&id);
    }

    #[inline]
    fn append_recv_fut_waker(&mut self, waker: Waker, id: ClientId) {
        self.recv_fut_wakers.push_back(Some((waker, id)));
    }

    #[inline]
    fn append_recv_sync_waker(&mut self) {
        // use condvar
        self.recv_fut_wakers.push_back(None);
    }
}

#[pin_project(PinnedDrop)]
struct Send<'a, T: DataDeliveryPolicy> {
    id: usize,
    channel: &'a Channel<T>,
    queued: bool,
    value: Option<T>,
}

#[pinned_drop]
#[allow(clippy::needless_lifetimes)]
impl<'a, T: DataDeliveryPolicy> PinnedDrop for Send<'a, T> {
    fn drop(self: Pin<&mut Self>) {
        if self.queued {
            self.channel.0.pc.lock().notify_send_fut_drop(self.id);
        }
    }
}

impl<'a, T> Future for Send<'a, T>
where
    T: DataDeliveryPolicy,
{
    type Output = Result<()>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut pc = self.channel.0.pc.lock();
        if self.queued {
            pc.confirm_send_fut_waked(self.id);
        }
        if pc.receivers == 0 {
            self.queued = false;
            return Poll::Ready(Err(Error::ChannelClosed));
        }
        if pc.send_fut_wakers.is_empty() || self.queued {
            let push_result = pc.queue.try_push(self.value.take().unwrap());
            if let Some(val) = push_result.value {
                self.value = Some(val);
            } else {
                self.queued = false;
                pc.notify_data_sent();
                return Poll::Ready(if push_result.pushed {
                    Ok(())
                } else {
                    Err(Error::ChannelSkipped)
                });
            }
        }
        self.queued = true;
        pc.append_send_fut_waker(cx.waker().clone(), self.id);
        Poll::Pending
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
    pub fn send(&self, value: T) -> impl Future<Output = Result<()>> + '_ {
        Send {
            id: self.channel.op_id(),
            channel: &self.channel,
            queued: false,
            value: Some(value),
        }
    }
    pub fn try_send(&self, value: T) -> Result<()> {
        let mut pc = self.channel.0.pc.lock();
        if pc.receivers == 0 {
            return Err(Error::ChannelClosed);
        }
        let push_result = pc.queue.try_push(value);
        if push_result.value.is_none() {
            pc.notify_data_sent();
            if push_result.pushed {
                Ok(())
            } else {
                Err(Error::ChannelSkipped)
            }
        } else {
            Err(Error::ChannelFull)
        }
    }
    pub fn send_blocking(&self, mut value: T) -> Result<()> {
        let mut pc = self.channel.0.pc.lock();
        let pushed = loop {
            if pc.receivers == 0 {
                return Err(Error::ChannelClosed);
            }
            let push_result = pc.queue.try_push(value);
            let Some(val) = push_result.value else {
                break push_result.pushed;
            };
            value = val;
            pc.append_send_sync_waker();
            self.channel.0.space_available.wait(&mut pc);
        };
        pc.wake_next_recv();
        if pushed {
            Ok(())
        } else {
            Err(Error::ChannelSkipped)
        }
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
            pc.wake_all_recvs();
        }
    }
}

struct Recv<'a, T: DataDeliveryPolicy> {
    id: usize,
    channel: &'a Channel<T>,
    queued: bool,
}

impl<'a, T: DataDeliveryPolicy> Drop for Recv<'a, T> {
    fn drop(&mut self) {
        if self.queued {
            self.channel.0.pc.lock().notify_recv_fut_drop(self.id);
        }
    }
}

impl<'a, T> Future for Recv<'a, T>
where
    T: DataDeliveryPolicy,
{
    type Output = Result<T>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut pc = self.channel.0.pc.lock();
        if self.queued {
            pc.confirm_recv_fut_waked(self.id);
        }
        if pc.recv_fut_wakers.is_empty() || self.queued {
            if let Some(val) = pc.queue.get() {
                pc.notify_data_received();
                self.queued = false;
                return Poll::Ready(Ok(val));
            } else if pc.senders == 0 {
                self.queued = false;
                return Poll::Ready(Err(Error::ChannelClosed));
            }
        }
        self.queued = true;
        pc.append_recv_fut_waker(cx.waker().clone(), self.id);
        Poll::Pending
    }
}

#[derive(Eq, PartialEq)]
pub struct Receiver<T>
where
    T: DataDeliveryPolicy,
{
    channel: Channel<T>,
}

impl<T> Receiver<T>
where
    T: DataDeliveryPolicy,
{
    #[inline]
    pub fn recv(&self) -> impl Future<Output = Result<T>> + '_ {
        Recv {
            id: self.channel.op_id(),
            channel: &self.channel,
            queued: false,
        }
    }
    pub fn try_recv(&self) -> Result<T> {
        let mut pc = self.channel.0.pc.lock();
        if let Some(val) = pc.queue.get() {
            pc.notify_data_received();
            Ok(val)
        } else if pc.senders == 0 {
            Err(Error::ChannelClosed)
        } else {
            Err(Error::ChannelEmpty)
        }
    }
    pub fn recv_blocking(&self) -> Result<T> {
        let mut pc = self.channel.0.pc.lock();
        loop {
            if let Some(val) = pc.queue.get() {
                pc.wake_next_send();
                return Ok(val);
            } else if pc.senders == 0 {
                return Err(Error::ChannelClosed);
            }
            pc.append_recv_sync_waker();
            self.channel.0.data_available.wait(&mut pc);
        }
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
            pc.wake_all_sends();
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

/// Creates a bounded async channel which respects [`DataDeliveryPolicy`] rules with no message
/// priority ordering
///
/// # Panics
///
/// Will panic if the capacity is zero
pub fn bounded<T: DataDeliveryPolicy>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let ch = Channel::new(capacity, false);
    make_channel(ch)
}

/// Creates a bounded async channel which respects [`DataDeliveryPolicy`] rules and has got message
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

    #[tokio::test]
    async fn test_delivery_policy_optional() {
        let (tx, rx) = bounded::<Message>(1);
        tokio::spawn(async move {
            for _ in 0..10 {
                tx.send(Message::Test(123)).await.unwrap();
                if let Err(e) = tx.send(Message::Spam).await {
                    assert!(e.is_data_skipped(), "{}", e);
                }
                tx.send(Message::Temperature(123.0)).await.unwrap();
            }
        });
        thread::sleep(Duration::from_secs(1));
        let mut messages = Vec::new();
        while let Ok(msg) = rx.recv().await {
            thread::sleep(Duration::from_millis(10));
            if matches!(msg, Message::Spam) {
                panic!("delivery policy not respected ({:?})", msg);
            }
            messages.push(msg);
        }
        insta::assert_debug_snapshot!(messages.len(), @"20");
    }

    #[tokio::test]
    async fn test_delivery_policy_single() {
        let (tx, rx) = bounded::<Message>(512);
        tokio::spawn(async move {
            for _ in 0..10 {
                tx.send(Message::Test(123)).await.unwrap();
                if let Err(e) = tx.send(Message::Spam).await {
                    assert!(e.is_data_skipped(), "{}", e);
                }
                tx.send(Message::Temperature(123.0)).await.unwrap();
            }
        });
        thread::sleep(Duration::from_secs(1));
        let mut c = 0;
        let mut t = 0;
        while let Ok(msg) = rx.recv().await {
            match msg {
                Message::Test(_) => c += 1,
                Message::Temperature(_) => t += 1,
                Message::Spam => {}
            }
        }
        insta::assert_snapshot!(c, @"10");
        insta::assert_snapshot!(t, @"1");
    }

    #[tokio::test]
    async fn test_sync_send_async_recv() {
        let (tx, rx) = bounded::<Message>(8);
        let tx_t = tx.clone();
        tokio::spawn(async move {
            for _ in 0..10 {
                tx.send(Message::Test(123)).await.unwrap();
                if let Err(e) = tx.send(Message::Spam).await {
                    assert!(e.is_data_skipped(), "{}", e);
                }
            }
        });
        tokio::task::spawn_blocking(move || {
            for _ in 0..10 {
                tx_t.send_blocking(Message::Test(123)).unwrap();
                if let Err(e) = tx_t.send_blocking(Message::Spam) {
                    assert!(e.is_data_skipped(), "{}", e);
                }
            }
        });
        thread::sleep(Duration::from_secs(1));
        let mut c = 0;
        while let Ok(msg) = rx.recv().await {
            if let Message::Test(_) = msg {
                c += 1;
            }
        }
        insta::assert_snapshot!(c, @"20");
    }
    #[tokio::test]
    async fn test_sync_send_sync_recv() {
        let (tx, rx) = bounded::<Message>(8);
        let tx_t = tx.clone();
        tokio::spawn(async move {
            for _ in 0..10 {
                tx.send(Message::Test(123)).await.unwrap();
                if let Err(e) = tx.send(Message::Spam).await {
                    assert!(e.is_data_skipped(), "{}", e);
                }
                tx.send(Message::Temperature(123.0)).await.unwrap();
            }
        });
        tokio::task::spawn_blocking(move || {
            for _ in 0..10 {
                tx_t.send_blocking(Message::Test(123)).unwrap();
                if let Err(e) = tx_t.send_blocking(Message::Spam) {
                    assert!(e.is_data_skipped(), "{}", e);
                }
                tx_t.send_blocking(Message::Temperature(123.0)).unwrap();
            }
        });
        thread::sleep(Duration::from_secs(1));
        let c = tokio::task::spawn_blocking(move || {
            let mut c = 0;
            while let Ok(msg) = rx.recv_blocking() {
                if let Message::Test(_) = msg {
                    c += 1;
                }
            }
            c
        })
        .await
        .unwrap();
        insta::assert_snapshot!(c, @"20");
    }

    #[tokio::test]
    async fn test_poisoning() {
        let n = 5_000;
        for _ in 0..n {
            let (tx, rx) = bounded::<Message>(512);
            let rx_t = tokio::spawn(async move { while rx.recv().await.is_ok() {} });
            tokio::spawn(async move {
                let _t = tx;
            });
            tokio::time::timeout(Duration::from_millis(100), rx_t)
                .await
                .unwrap()
                .unwrap();
        }
    }
}
