use binrw::BinWrite;
use busrt::rpc::{RpcError, RpcEvent, RpcHandlers, RpcResult};
use busrt::{async_trait, QoS};
use core::fmt;
pub use eva_common::acl::OIDMask;
use eva_common::common_payloads::ParamsId;
use eva_common::events::{RawStateEventOwned, RAW_STATE_TOPIC};
use eva_common::payload::{pack, unpack};
use eva_common::value::{to_value, Value};
pub use eva_common::OID;
use eva_sdk::controller::format_action_topic;
pub use eva_sdk::controller::Action;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Cursor;
use std::mem;
use std::sync::Arc;
use std::time::Duration;

use crate::controller::{Context, SLEEP_STEP};
use crate::{pchannel_async, DataDeliveryPolicy, DeliveryPolicy};
use crate::{
    pchannel_async::{Receiver as ReceiverAsync, Sender as SenderAsync},
    Error, Result,
};
use busrt::{
    ipc::Client,
    rpc::{Rpc, RpcClient},
};
use tracing::{error, info, warn};

enum PushPayload {
    State {
        oid: Arc<OID>,
        event: RawStateEventOwned,
    },
    DObj {
        name: Arc<String>,
        data: Vec<u8>,
    },
    DObjError(Arc<String>),
    ActionState {
        topic: Arc<String>,
        payload: Vec<u8>,
    },
}

impl DataDeliveryPolicy for PushPayload {
    fn delivery_policy(&self) -> DeliveryPolicy {
        DeliveryPolicy::Single
    }
    fn priority(&self) -> usize {
        100
    }
    fn eq_kind(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::State { oid: a, .. }, Self::State { oid: b, .. }) => a == b,
            (Self::DObj { name: a, .. }, Self::DObj { name: b, .. }) => a == b,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EAPIConfig<D, V>
where
    D: DataDeliveryPolicy + Clone + Send + Sync + 'static,
    V: Send,
{
    path: String,
    timeout: Option<f64>,
    buf_size: Option<usize>,
    queue_size: Option<usize>,
    buf_ttl: Option<u64>,
    reconnect_delay: f64,
    #[serde(skip)]
    action_handlers: BTreeMap<OID, ActionHandlerFn<D, V>>,
    #[serde(skip)]
    bulk_action_handlers: Vec<(OIDMask, ActionHandlerFn<D, V>)>,
}

impl<D, V> EAPIConfig<D, V>
where
    D: DataDeliveryPolicy + Clone + Send + Sync + 'static,
    V: Send,
{
    fn to_busrt_config(&self, name: &str) -> busrt::ipc::Config {
        let mut config = busrt::ipc::Config::new(&self.path, name);
        if let Some(timeout) = self.timeout {
            config = config.timeout(Duration::from_secs_f64(timeout));
        }
        if let Some(buf_size) = self.buf_size {
            config = config.buf_size(buf_size);
        }
        if let Some(queue_size) = self.queue_size {
            config = config.queue_size(queue_size);
        }
        if let Some(buf_ttl) = self.buf_ttl {
            config = config.buf_ttl(Duration::from_micros(buf_ttl));
        }
        config
    }
    pub fn new(path: &str) -> Self {
        Self {
            path: path.to_owned(),
            timeout: None,
            buf_size: None,
            queue_size: None,
            buf_ttl: None,
            reconnect_delay: 2.0,
            action_handlers: <_>::default(),
            bulk_action_handlers: <_>::default(),
        }
    }
    /// Set timeout in seconds
    pub fn timeout(mut self, timeout: f64) -> Self {
        self.timeout = Some(timeout);
        self
    }
    /// Set buffer size
    pub fn buf_size(mut self, buf_size: usize) -> Self {
        self.buf_size = Some(buf_size);
        self
    }
    /// Set queue size
    pub fn queue_size(mut self, queue_size: usize) -> Self {
        self.queue_size = Some(queue_size);
        self
    }
    /// Set buffer TTL (in microseconds)
    pub fn buf_ttl(mut self, buf_ttl: u64) -> Self {
        self.buf_ttl = Some(buf_ttl);
        self
    }
    /// Set reconnect delay in seconds
    pub fn reconnect_delay(mut self, reconnect_delay: f64) -> Self {
        self.reconnect_delay = reconnect_delay;
        self
    }
    pub fn action_handler(mut self, oid: OID, handler: ActionHandlerFn<D, V>) -> Self {
        self.action_handlers.insert(oid, handler);
        self
    }
    pub fn bulk_action_handler(mut self, mask: OIDMask, handler: ActionHandlerFn<D, V>) -> Self {
        self.bulk_action_handlers.push((mask, handler));
        self
    }
}

pub type ActionHandlerFn<D, V> = fn(&mut Action, context: &Context<D, V>) -> ActionResult;
pub type ActionResult = std::result::Result<(), Box<dyn std::error::Error>>;

type ActionHandlers<D, V> = Arc<BTreeMap<OID, ActionHandlerFn<D, V>>>;
type BulkActionHandlers<D, V> = Arc<Vec<(OIDMask, ActionHandlerFn<D, V>)>>;

#[allow(clippy::struct_field_names)]
struct Handlers<D, V>
where
    D: DataDeliveryPolicy + Clone + Send + Sync + 'static,
    V: Send,
{
    action_handlers: ActionHandlers<D, V>,
    bulk_action_handlers: BulkActionHandlers<D, V>,
    tx: SenderAsync<PushPayload>,
    context: Context<D, V>,
}

fn handle_action<D, V>(
    action: &mut Action,
    topic: Arc<String>,
    tx: SenderAsync<PushPayload>,
    action_handlers: ActionHandlers<D, V>,
    bulk_action_handlers: BulkActionHandlers<D, V>,
    context: &Context<D, V>,
) -> ActionResult
where
    D: DataDeliveryPolicy + Clone + Send + Sync + 'static,
    V: Send,
{
    macro_rules! notify_running {
        () => {
            if let Ok(payload) = pack(&action.event_running()) {
                let _ = tx.try_send(PushPayload::ActionState { topic, payload });
            }
        };
    }
    if let Some(handler) = action_handlers.get(action.oid()) {
        notify_running!();
        return handler(action, context);
    }
    for (mask, handler) in bulk_action_handlers.iter() {
        if mask.matches(action.oid()) {
            notify_running!();
            return handler(action, context);
        }
    }
    Err(eva_common::Error::not_found(format!("action handler not found: {}", action.oid())).into())
}

#[async_trait]
impl<D, V> RpcHandlers for Handlers<D, V>
where
    D: DataDeliveryPolicy + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
{
    async fn handle_call(&self, event: RpcEvent) -> RpcResult {
        let payload = event.payload();
        match event.parse_method()? {
            "test" => {
                if payload.is_empty() {
                    Ok(None)
                } else {
                    Err(RpcError::params(None))
                }
            }
            "action" | "run" => {
                if payload.is_empty() {
                    return Err(RpcError::params(None));
                }
                let mut action: Action = unpack(payload)?;
                let action_handlers = self.action_handlers.clone();
                let bulk_action_handlers = self.bulk_action_handlers.clone();
                let tx = self.tx.clone();
                let context = self.context.clone();
                tokio::task::spawn_blocking(move || {
                    let topic = Arc::new(format_action_topic(action.oid()));
                    let payload = if let Err(e) = handle_action(
                        &mut action,
                        topic.clone(),
                        tx.clone(),
                        action_handlers,
                        bulk_action_handlers,
                        &context,
                    ) {
                        action.event_failed(1, None, Some(Value::String(e.to_string())))
                    } else {
                        action.event_completed(None)
                    };
                    match pack(&payload) {
                        Ok(packed) => {
                            if let Err(error) = tx.send_blocking(PushPayload::ActionState {
                                topic,
                                payload: packed,
                            }) {
                                error!(%error, "failed to send action state");
                            }
                        }
                        Err(e) => error!("action payload pack failed: {}", e),
                    }
                })
                .await
                .map_err(eva_common::Error::failed)?;
                Ok(None)
            }

            _ => Err(RpcError::method(None)),
        }
    }
}

pub struct EAPI<D, V>
where
    D: DataDeliveryPolicy + Clone + Send + Sync + 'static,
    V: Send,
{
    inner: Arc<EAPIInner<D, V>>,
}

impl<D, V> Clone for EAPI<D, V>
where
    D: DataDeliveryPolicy + Clone + Send + Sync + 'static,
    V: Send,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

struct EAPIInner<D, V>
where
    D: DataDeliveryPolicy + Clone + Send + Sync + 'static,
    V: Send,
{
    name: String,
    config: EAPIConfig<D, V>,
    tx: SenderAsync<PushPayload>,
    rx: ReceiverAsync<PushPayload>,
    action_handlers: ActionHandlers<D, V>,
    bulk_action_handlers: BulkActionHandlers<D, V>,
}

impl<D, V> EAPI<D, V>
where
    D: DataDeliveryPolicy + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
{
    pub fn new<N: fmt::Display>(name: N, mut config: EAPIConfig<D, V>) -> Self {
        let (tx, rx) =
            pchannel_async::bounded(config.queue_size.unwrap_or(busrt::DEFAULT_QUEUE_SIZE));
        let action_handlers = mem::take(&mut config.action_handlers);
        let bulk_action_handlers = mem::take(&mut config.bulk_action_handlers);
        Self {
            inner: EAPIInner {
                name: name.to_string(),
                config,
                tx,
                rx,
                action_handlers: Arc::new(action_handlers),
                bulk_action_handlers: Arc::new(bulk_action_handlers),
            }
            .into(),
        }
    }
    /// # Panics
    ///
    /// Will panic if failed to start the tokio runtime
    pub fn run(&self, thread_name: &str, context: &Context<D, V>) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .thread_name(thread_name)
            .build()
            .unwrap();
        rt.block_on(self.run_async(context));
    }
    async fn run_async(&self, context: &Context<D, V>) {
        let reconnect_delay = Duration::from_secs_f64(self.inner.config.reconnect_delay);
        loop {
            if let Err(err) = self.bus(context).await {
                error!(client=self.inner.name, %err, "failed to connect to EAPI bus");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            tokio::time::sleep(reconnect_delay).await;
        }
    }
    async fn bus(&self, context: &Context<D, V>) -> Result<()> {
        let bus_config = self.inner.config.to_busrt_config(&self.inner.name);
        let client = Client::connect(&bus_config).await.map_err(Error::io)?;
        info!(
            client = self.inner.name,
            path = self.inner.config.path,
            "connected to EAPI bus"
        );
        let handlers = Handlers {
            tx: self.inner.tx.clone(),
            action_handlers: self.inner.action_handlers.clone(),
            bulk_action_handlers: self.inner.bulk_action_handlers.clone(),
            context: context.clone(),
        };
        let rpc = Arc::new(RpcClient::new(client, handlers));
        let rpc_c = rpc.clone();
        let rx = self.inner.rx.clone();
        let push_worker = tokio::spawn(async move {
            while let Ok(payload) = rx.recv().await {
                match payload {
                    PushPayload::State { oid, event } => {
                        let topic = format!("{}{}", RAW_STATE_TOPIC, oid.as_path());
                        match pack(&event) {
                            Ok(data) => {
                                if let Err(e) = rpc_c
                                    .client()
                                    .lock()
                                    .await
                                    .publish(&topic, data.into(), QoS::Realtime)
                                    .await
                                {
                                    error!(%e, "failed to publish state event");
                                }
                            }
                            Err(err) => {
                                error!(%err, "failed to pack state event");
                            }
                        }
                    }
                    PushPayload::DObj { name, data } => {
                        #[derive(Serialize)]
                        struct DobjPushPayload<'a> {
                            i: &'a str,
                            d: &'a [u8],
                        }
                        match pack(&DobjPushPayload { i: &name, d: &data }) {
                            Ok(data) => {
                                if let Err(e) = rpc_c
                                    .call("eva.core", "dobj.push", data.into(), QoS::Realtime)
                                    .await
                                {
                                    error!(%e, "failed to publish dobj");
                                }
                            }
                            Err(err) => {
                                error!(%err, "failed to pack dobj");
                            }
                        }
                    }
                    PushPayload::DObjError(name) => match pack(&ParamsId { i: &name }) {
                        Ok(data) => {
                            if let Err(e) = rpc_c
                                .call("eva.core", "dobj.error", data.into(), QoS::Realtime)
                                .await
                            {
                                error!(%e, "failed to publish dobj error");
                            }
                        }
                        Err(err) => {
                            error!(%err, "failed to pack dobj error");
                        }
                    },
                    PushPayload::ActionState { topic, payload } => {
                        if let Err(e) = rpc_c
                            .client()
                            .lock()
                            .await
                            .publish(&topic, payload.into(), QoS::Realtime)
                            .await
                        {
                            error!(%e, "failed to publish action state");
                        }
                    }
                }
            }
        });
        while rpc.client().lock().await.is_connected() {
            tokio::time::sleep(SLEEP_STEP).await;
        }
        push_worker.abort();
        warn!(client = self.inner.name, "disconnected from EAPI bus");
        Ok(())
    }
    pub fn dobj_push<T>(&self, name: Arc<String>, value: T) -> Result<()>
    where
        T: for<'a> BinWrite<Args<'a> = ()>,
    {
        let mut data = Cursor::new(vec![]);
        value.write_le(&mut data)?;
        self.inner.tx.try_send(PushPayload::DObj {
            name: name.clone(),
            data: data.into_inner(),
        })
    }
    pub fn dobj_error(&self, name: Arc<String>) -> Result<()> {
        self.inner.tx.try_send(PushPayload::DObjError(name))
    }
    pub fn state_push<T: Serialize>(&self, oid: Arc<OID>, value: T) -> Result<()> {
        self.inner.tx.try_send(PushPayload::State {
            oid,
            event: RawStateEventOwned::new(1, to_value(value).map_err(Error::invalid_data)?),
        })
    }
    pub fn state_error(&self, oid: Arc<OID>) -> Result<()> {
        self.inner.tx.try_send(PushPayload::State {
            oid,
            event: RawStateEventOwned::new0(eva_common::ITEM_STATUS_ERROR),
        })
    }
}
