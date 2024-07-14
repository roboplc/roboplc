use std::future::Future;
use std::sync::Arc;

use crate::locking::Mutex;

use crate::pchannel_async::{self, Receiver, Sender};
use crate::{DataDeliveryPolicy, Error, Result};

type ConditionFunction<T> = Box<dyn Fn(&T) -> bool + Send + Sync>;

/// The default priority for the client channel
pub const DEFAULT_PRIORITY: usize = 100;

/// The default client channel capacity
pub const DEFAULT_CHANNEL_CAPACITY: usize = 1024;

/// Async data communcation hub to implement in-process pub/sub model for thread workers
pub struct Hub<T: DataDeliveryPolicy + Clone> {
    inner: Arc<Mutex<HubInner<T>>>,
}

impl<T: DataDeliveryPolicy + Clone> Clone for Hub<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T: DataDeliveryPolicy + Clone> Default for Hub<T> {
    fn default() -> Self {
        Self {
            inner: <_>::default(),
        }
    }
}

impl<T: DataDeliveryPolicy + Clone> Hub<T> {
    /// Creates a new hub instance
    pub fn new() -> Self {
        Self::default()
    }
    /// Sets the default client channel capacity (the default is 1024), can be used as a build
    /// pattern
    pub fn set_default_channel_capacity(self, capacity: usize) -> Self {
        self.inner.lock().default_channel_capacity = capacity;
        self
    }
    /// Sends a message to subscribed clients, ignores send errors
    ///
    /// # Panics
    ///
    /// Should not panic
    pub async fn send(&self, message: T) {
        macro_rules! send {
            ($sub: expr, $msg: expr) => {
                let _r = $sub.tx.send($msg).await;
            };
        }
        // clones matching subscribers to keep the internal mutex unlocked and avoid deadlocks
        let targets: Vec<Arc<Subscription<T>>> = self
            .inner
            .lock()
            .subscriptions
            .iter()
            .filter(|c| (c.condition)(&message))
            .cloned()
            .collect();
        if targets.is_empty() {
            return;
        }
        for sub in targets.iter().take(targets.len() - 1) {
            if (sub.condition)(&message) {
                send!(sub, message.clone());
            }
        }
        let sub = targets.last().unwrap();
        if (sub.condition)(&message) {
            send!(sub, message);
        }
    }
    /// Sends a message to subscribed clients, calls an error handlers function in case of errors
    /// with some subsciber
    ///
    /// If the error function returns false, the whole operation is aborted
    ///
    /// # Panics
    ///
    /// Should not panic
    pub async fn send_checked<F>(&self, message: T, error_handler: F) -> Result<()>
    where
        F: Fn(&str, &Error) -> bool,
    {
        macro_rules! send_checked {
            ($sub: expr, $msg: expr) => {
                if let Err(e) = $sub.tx.send($msg).await {
                    let err = e.into();
                    if !error_handler(&$sub.name, &err) {
                        return Err(Error::HubSend(err.into()));
                    }
                }
            };
        }
        let targets: Vec<Arc<Subscription<T>>> = self
            .inner
            .lock()
            .subscriptions
            .iter()
            .filter(|c| (c.condition)(&message))
            .cloned()
            .collect();
        if targets.is_empty() {
            return Ok(());
        }
        for sub in targets.iter().take(targets.len() - 1) {
            if (sub.condition)(&message) {
                send_checked!(sub, message.clone());
            }
        }
        let sub = targets.last().unwrap();
        if (sub.condition)(&message) {
            send_checked!(sub, message);
        }
        Ok(())
    }
    /// Registers a sender-only client with no subscriptions
    ///
    /// If attempting to receive a message from such client, [`Error::ChannelClosed`] is returned
    pub fn sender(&self) -> Client<T> {
        let (_, rx) = pchannel_async::bounded(1);
        Client {
            name: "".into(),
            hub: self.clone(),
            rx,
        }
    }
    /// Registers a regular client. The condition function is used to check which kinds of
    /// messages should be delivered (returns true for subscribed)
    pub fn register<F>(&self, name: &str, condition: F) -> Result<Client<T>>
    where
        F: Fn(&T) -> bool + Send + Sync + 'static,
    {
        self.register_with_options(ClientOptions::new(name, condition))
    }
    /// Registers a regular client with custom options
    pub fn register_with_options(&self, client_options: ClientOptions<T>) -> Result<Client<T>> {
        let name = client_options.name.clone();
        let mut inner = self.inner.lock();
        if inner.subscriptions.iter().any(|client| client.name == name) {
            return Err(Error::HubAlreadyRegistered(name));
        }
        let capacity = client_options
            .capacity
            .unwrap_or(inner.default_channel_capacity);
        let (tx, rx) = if client_options.ordering {
            pchannel_async::ordered(capacity)
        } else {
            pchannel_async::bounded(capacity)
        };
        inner
            .subscriptions
            .push(client_options.into_subscription(tx).into());
        inner
            .subscriptions
            .sort_by(|a, b| a.priority.cmp(&b.priority));
        Ok(Client {
            name,
            hub: self.clone(),
            rx,
        })
    }
    fn unregister(&self, name: &str) {
        self.inner
            .lock()
            .subscriptions
            .retain(|client| &*client.name != name);
    }
}

struct HubInner<T: DataDeliveryPolicy + Clone> {
    default_channel_capacity: usize,
    subscriptions: Vec<Arc<Subscription<T>>>,
}

impl<T> Default for HubInner<T>
where
    T: DataDeliveryPolicy + Clone,
{
    fn default() -> Self {
        Self {
            default_channel_capacity: DEFAULT_CHANNEL_CAPACITY,
            subscriptions: <_>::default(),
        }
    }
}

/// A client for the hub
pub struct Client<T: DataDeliveryPolicy + Clone> {
    name: Arc<str>,
    hub: Hub<T>,
    rx: Receiver<T>,
}

impl<T: DataDeliveryPolicy + Clone> Client<T> {
    /// Sends a message to hub-subscribed clients, ignores send errors
    pub fn send(&self, message: T) -> impl Future<Output = ()> + '_ {
        self.hub.send(message)
    }
    /// Sends a message to subscribed clients, calls an error handlers function in case of errors
    /// with some subsciber
    ///
    /// If the error function returns false, the whole operation is aborted
    pub fn send_checked<'a, F>(
        &'a self,
        message: T,
        error_handler: F,
    ) -> impl Future<Output = Result<()>> + 'a
    where
        F: Fn(&str, &Error) -> bool + 'a,
    {
        self.hub.send_checked(message, error_handler)
    }
    /// Receives a message from the hub (blocking)
    pub fn recv(&self) -> impl Future<Output = rtsc::Result<T>> + '_ {
        self.rx.recv()
    }
    /// Receives a message from the hub (non-blocking)
    pub fn try_recv(&self) -> rtsc::Result<T> {
        self.rx.try_recv()
    }
}

impl<T: DataDeliveryPolicy + Clone> Drop for Client<T> {
    fn drop(&mut self) {
        self.hub.unregister(&self.name);
    }
}

/// Client options
pub struct ClientOptions<T: DataDeliveryPolicy + Clone> {
    name: Arc<str>,
    priority: usize,
    capacity: Option<usize>,
    ordering: bool,
    condition: ConditionFunction<T>,
}

impl<T: DataDeliveryPolicy + Clone> ClientOptions<T> {
    /// Creates a new client options object
    pub fn new<F>(name: &str, condition: F) -> Self
    where
        F: Fn(&T) -> bool + Send + Sync + 'static,
    {
        Self {
            name: name.to_owned().into(),
            priority: DEFAULT_PRIORITY,
            capacity: None,
            ordering: false,
            condition: Box::new(condition),
        }
    }
    /// Enables client channel priority ordering
    pub fn ordering(mut self, ordering: bool) -> Self {
        self.ordering = ordering;
        self
    }
    /// Sets client priority (the default is 100)
    pub fn priority(mut self, priority: usize) -> Self {
        self.priority = priority;
        self
    }
    /// Overrides the default hub client channel capacity
    pub fn capacity(mut self, capacity: usize) -> Self {
        self.capacity = Some(capacity);
        self
    }
    fn into_subscription(self, tx: Sender<T>) -> Subscription<T> {
        Subscription {
            name: self.name,
            tx,
            priority: self.priority,
            condition: self.condition,
        }
    }
}

/// A macro which can be used to match an event with enum for [`Hub`] subscription condition
///
/// # Examples
///
/// ```rust
/// use roboplc::event_matches;
///
/// enum Message {
///     Temperature(f64),
///     Flush,
///     Other
/// }
///
/// let condition_fn = event_matches!(Message::Temperature(_) | (Message::Flush));
/// ```

struct Subscription<T: DataDeliveryPolicy + Clone> {
    name: Arc<str>,
    tx: Sender<T>,
    priority: usize,
    condition: ConditionFunction<T>,
}

#[cfg(test)]
mod test {
    use crate::{event_matches, DataDeliveryPolicy};

    use super::Hub;

    #[derive(Clone, Debug)]
    enum Message {
        Temperature(f64),
        Humidity(f64),
        Test,
    }

    impl DataDeliveryPolicy for Message {}

    #[tokio::test]
    async fn test_hub() {
        let hub = Hub::<Message>::new().set_default_channel_capacity(20);
        let sender = hub.sender();
        let client1 = hub
            .register(
                "test1",
                event_matches!(Message::Temperature(_) | Message::Humidity(_)),
            )
            .unwrap();
        for _ in 0..3 {
            sender.send(Message::Temperature(1.0)).await;
            sender.send(Message::Humidity(2.0)).await;
            sender.send(Message::Test).await;
        }
        let mut messages = Vec::with_capacity(20);
        while let Ok(msg) = client1.try_recv() {
            messages.push(msg);
        }
        insta::assert_snapshot!(messages.len(), @"6");
        insta::assert_debug_snapshot!(messages);
    }
}
