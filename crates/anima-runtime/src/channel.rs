pub mod adapter;
pub(crate) mod dispatch;
pub(crate) mod http;
pub(crate) mod message;
pub(crate) mod rabbitmq;
pub(crate) mod registry;
pub mod session;

pub use adapter::{
    err, ok, success, error, Channel, SendOptions, SendResult, SentMessage, StreamingChannel,
    TestChannel,
};
pub use dispatch::{
    dispatch_outbound_message, start_outbound_dispatch, DispatchStats, DispatchStatsSnapshot,
};
pub use http::{
    ChannelAdapter, ChannelCapabilities, ChannelHealthSnapshot, HttpChannel, HttpChannelConfig,
};
pub use message::{
    create_message, extract_session_id, extract_user_id, make_channel_routing_key,
    make_session_routing_key, make_user_routing_key, ChannelMessage, CreateMessage,
    BROADCAST_ROUTING_KEY,
};
pub use rabbitmq::{RabbitMqChannel, RabbitMqChannelConfig};
pub use registry::{
    ChannelHealth, ChannelLookupReason, ChannelLookupSnapshot, ChannelRegistry,
    ChannelRegistryEntry, HealthReport,
};
pub use session::{
    FindSessionOptions, Session, SessionCreateOptions, SessionStats, SessionStore,
};
