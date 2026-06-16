//! NATS client for task subscription and heartbeat publishing.

use async_trait::async_trait;
use futures::Stream;

use crate::messages::HeartbeatMessage;

/// Trait for NATS operations — allows test doubles and fakes.
#[async_trait]
pub trait NatsClient: Send + Sync + 'static {
    /// Subscribe to task updates for a region.
    ///
    /// Returns a `Stream` of raw `async_nats::Message`. The caller is responsible
    /// for deserializing the payload.
    async fn subscribe(
        &self,
        region: &str,
    ) -> anyhow::Result<Box<dyn Stream<Item = async_nats::Message> + Send + Unpin>>;

    /// Publish a heartbeat message to the given region.
    async fn publish_heartbeat(&self, region: &str, msg: &HeartbeatMessage) -> anyhow::Result<()>;
}

/// Production NATS client wrapping async-nats.
pub struct NatsClientImpl {
    client: async_nats::Client,
}

impl NatsClientImpl {
    /// Connect to a NATS server.
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let client = async_nats::connect(url).await?;
        Ok(Self { client })
    }
}

#[async_trait]
impl NatsClient for NatsClientImpl {
    async fn subscribe(
        &self,
        region: &str,
    ) -> anyhow::Result<Box<dyn Stream<Item = async_nats::Message> + Send + Unpin>> {
        let subject = format!("edgecloud.tasks.{}", region);
        let subscription = self.client.subscribe(subject).await?;
        Ok(Box::new(subscription))
    }

    async fn publish_heartbeat(&self, region: &str, msg: &HeartbeatMessage) -> anyhow::Result<()> {
        let subject = format!("edgecloud.heartbeats.{}", region);
        let payload = serde_json::to_vec(msg)?;
        self.client.publish(subject, payload.into()).await?;
        Ok(())
    }
}
