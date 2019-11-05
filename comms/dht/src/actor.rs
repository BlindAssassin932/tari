// Copyright 2019, The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
// disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
// following disclaimer in the documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
// products derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
// WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
// USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

//! Actor for DHT functionality.
//!
//! The DhtActor is responsible for sending a join request on startup
//! and furnishing [DhtRequest]s.
//!
//! [DhtRequest]: ./enum.DhtRequest.html

use crate::{
    envelope::NodeDestination,
    outbound::{
        BroadcastClosestRequest,
        BroadcastStrategy,
        DhtOutboundError,
        OutboundEncryption,
        OutboundMessageRequester,
    },
    proto::{
        dht::{DiscoverMessage, JoinMessage},
        envelope::DhtMessageType,
        store_forward::StoredMessagesRequest,
    },
    DhtConfig,
};
use derive_error::Error;
use futures::{
    channel::{mpsc, mpsc::SendError, oneshot},
    stream::Fuse,
    FutureExt,
    SinkExt,
    StreamExt,
};
use log::*;
use std::sync::Arc;
use tari_comms::{
    peer_manager::{NodeId, NodeIdentity},
    types::CommsPublicKey,
};
use tari_shutdown::ShutdownSignal;
use tari_utilities::ByteArray;
use ttl_cache::TtlCache;

const LOG_TARGET: &'static str = "comms::dht::actor";

#[derive(Debug, Error)]
pub enum DhtActorError {
    /// MPSC channel is disconnected
    ChannelDisconnected,
    /// MPSC sender was unable to send because the channel buffer is full
    SendBufferFull,
    /// Reply sender canceled the request
    ReplyCanceled,
}

impl From<SendError> for DhtActorError {
    fn from(err: SendError) -> Self {
        if err.is_disconnected() {
            DhtActorError::ChannelDisconnected
        } else if err.is_full() {
            DhtActorError::SendBufferFull
        } else {
            unreachable!();
        }
    }
}

#[derive(Debug)]
pub enum DhtRequest {
    /// Send a Join request to the network
    SendJoin,
    /// Send a discover request for a network region or node
    SendDiscover {
        dest_public_key: CommsPublicKey,
        dest_node_id: Option<NodeId>,
        destination: NodeDestination,
    },
    /// Inserts a message signature to the signature cache. This operation replies with a boolean
    /// which is true if the signature already exists in the cache, otherwise false
    SignatureCacheInsert(Box<Vec<u8>>, oneshot::Sender<bool>),
}

#[derive(Clone)]
pub struct DhtRequester {
    sender: mpsc::Sender<DhtRequest>,
}

impl DhtRequester {
    pub fn new(sender: mpsc::Sender<DhtRequest>) -> Self {
        Self { sender }
    }

    pub async fn send_join(&mut self) -> Result<(), DhtActorError> {
        self.sender.send(DhtRequest::SendJoin).await.map_err(Into::into)
    }

    pub async fn send_discover(
        &mut self,
        dest_public_key: CommsPublicKey,
        dest_node_id: Option<NodeId>,
        destination: NodeDestination,
    ) -> Result<(), DhtActorError>
    {
        self.sender
            .send(DhtRequest::SendDiscover {
                dest_public_key,
                dest_node_id,
                destination,
            })
            .await
            .map_err(Into::into)
    }

    pub async fn insert_message_signature(&mut self, signature: Vec<u8>) -> Result<bool, DhtActorError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DhtRequest::SignatureCacheInsert(Box::new(signature), reply_tx))
            .await?;

        reply_rx.await.map_err(|_| DhtActorError::ReplyCanceled)
    }
}

pub struct DhtActor {
    node_identity: Arc<NodeIdentity>,
    outbound_requester: OutboundMessageRequester,
    config: DhtConfig,
    shutdown_signal: Option<ShutdownSignal>,
    request_rx: Fuse<mpsc::Receiver<DhtRequest>>,
    signature_cache: TtlCache<Vec<u8>, ()>,
}

impl DhtActor {
    pub fn new(
        config: DhtConfig,
        node_identity: Arc<NodeIdentity>,
        outbound_requester: OutboundMessageRequester,
        request_rx: mpsc::Receiver<DhtRequest>,
        shutdown_signal: ShutdownSignal,
    ) -> Self
    {
        Self {
            signature_cache: TtlCache::new(config.signature_cache_capacity),
            config,
            outbound_requester,
            node_identity,
            shutdown_signal: Some(shutdown_signal),
            request_rx: request_rx.fuse(),
        }
    }

    pub async fn start(mut self) {
        if self.config.enable_auto_join {
            match self.send_join().await {
                Ok(_) => {
                    trace!(target: LOG_TARGET, "Join message has been sent to closest peers",);
                },
                Err(err) => {
                    error!(
                        target: LOG_TARGET,
                        "Failed to send join message on startup because '{}'", err
                    );
                },
            }
        }

        if self.config.enable_auto_stored_message_request {
            match self.request_stored_messages().await {
                Ok(_) => {
                    trace!(
                        target: LOG_TARGET,
                        "Stored message request has been sent to closest peers",
                    );
                },
                Err(err) => {
                    error!(
                        target: LOG_TARGET,
                        "Failed to send stored message on startup because '{}'", err
                    );
                },
            }
        }

        let mut shutdown_signal = self
            .shutdown_signal
            .take()
            .expect("DhtActor initialized without shutdown_signal")
            .fuse();

        loop {
            futures::select! {
                request = self.request_rx.select_next_some() => {
                    debug!(target: LOG_TARGET, "DHtActor received message: {:?}", request);
                    self.handle_request(request).await;
                },

                _guard = shutdown_signal => {
                    info!(target: LOG_TARGET, "DHtActor is shutting down because it received a shutdown signal.");
                    break;
                },
                complete => {
                    info!(target: LOG_TARGET, "DHtActor is shutting down because the request stream ended.");
                    break;
                }
            }
        }
    }

    async fn handle_request(&mut self, request: DhtRequest) {
        use DhtRequest::*;
        let result = match request {
            SendJoin => self.send_join().await,
            SendDiscover {
                destination,
                dest_node_id,
                dest_public_key,
            } => self.send_discover(dest_public_key, dest_node_id, destination).await,

            SignatureCacheInsert(signature, reply_tx) => {
                let already_exists = self
                    .signature_cache
                    .insert(*signature, (), self.config.signature_cache_ttl)
                    .is_some();
                let _ = reply_tx.send(already_exists);
                Ok(())
            },
        };

        match result {
            Ok(_) => {
                trace!(target: LOG_TARGET, "Successfully handled DHT request message");
            },
            Err(err) => {
                error!(target: LOG_TARGET, "Error when handling DHT request message. {}", err);
            },
        }
    }

    async fn send_join(&mut self) -> Result<(), DhtOutboundError> {
        let message = JoinMessage {
            node_id: self.node_identity.node_id().to_vec(),
            addresses: vec![self.node_identity.control_service_address().to_string()],
            peer_features: self.node_identity.features().bits(),
        };

        debug!(
            target: LOG_TARGET,
            "Sending Join message to (at most) {} closest peers", self.config.num_neighbouring_nodes
        );

        self.outbound_requester
            .send_dht_message(
                BroadcastStrategy::Closest(Box::new(BroadcastClosestRequest {
                    n: self.config.num_neighbouring_nodes,
                    node_id: self.node_identity.node_id().clone(),
                    excluded_peers: Vec::new(),
                })),
                NodeDestination::Unknown,
                OutboundEncryption::None,
                DhtMessageType::Join,
                message,
            )
            .await?;

        Ok(())
    }

    async fn send_discover(
        &mut self,
        dest_public_key: CommsPublicKey,
        dest_node_id: Option<NodeId>,
        destination: NodeDestination,
    ) -> Result<(), DhtOutboundError>
    {
        let discover_msg = DiscoverMessage {
            node_id: self.node_identity.node_id().to_vec(),
            addresses: vec![self.node_identity.control_service_address().to_string()],
            peer_features: self.node_identity.features().bits(),
        };
        debug!(
            target: LOG_TARGET,
            "Sending Discover message to (at most) {} closest peers", self.config.num_neighbouring_nodes
        );

        // If the destination node is is known, send to the closest peers we know. Otherwise...
        let network_location_node_id = dest_node_id.unwrap_or(match &destination {
            // ... if the destination is undisclosed or a public key, send discover to our closest peers
            NodeDestination::Unknown | NodeDestination::PublicKey(_) => self.node_identity.node_id().clone(),
            // otherwise, send it to the closest peers to the given NodeId destination we know
            NodeDestination::NodeId(node_id) => node_id.clone(),
        });

        let broadcast_strategy = BroadcastStrategy::Closest(Box::new(BroadcastClosestRequest {
            n: self.config.num_neighbouring_nodes,
            node_id: network_location_node_id,
            excluded_peers: Vec::new(),
        }));

        self.outbound_requester
            .send_dht_message(
                broadcast_strategy,
                destination,
                OutboundEncryption::EncryptFor(dest_public_key),
                DhtMessageType::Discover,
                discover_msg,
            )
            .await?;

        Ok(())
    }

    async fn request_stored_messages(&mut self) -> Result<(), DhtOutboundError> {
        let broadcast_strategy = BroadcastStrategy::Closest(Box::new(BroadcastClosestRequest {
            n: self.config.num_neighbouring_nodes,
            node_id: self.node_identity.node_id().clone(),
            excluded_peers: Vec::new(),
        }));

        self.outbound_requester
            .send_dht_message(
                broadcast_strategy,
                NodeDestination::Unknown,
                OutboundEncryption::EncryptForDestination,
                DhtMessageType::SafRequestMessages,
                // TODO: We should track when this node last requested stored messages and ask
                //       for messages after that date
                StoredMessagesRequest::new(),
            )
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_utils::make_node_identity;
    use tari_shutdown::Shutdown;
    use tari_test_utils::runtime;

    #[test]
    fn auto_messages() {
        runtime::test_async(|rt| {
            let node_identity = make_node_identity();
            let (out_tx, mut out_rx) = mpsc::channel(1);
            let (_actor_tx, actor_rx) = mpsc::channel(1);
            let outbound_requester = OutboundMessageRequester::new(out_tx);
            let shutdown = Shutdown::new();
            let actor = DhtActor::new(
                DhtConfig::default(),
                node_identity,
                outbound_requester,
                actor_rx,
                shutdown.to_signal(),
            );

            rt.spawn(actor.start());

            rt.block_on(async move {
                let request = unwrap_oms_send_msg!(out_rx.next().await.unwrap());
                assert_eq!(request.dht_message_type, DhtMessageType::Join);
                let request = unwrap_oms_send_msg!(out_rx.next().await.unwrap());
                assert_eq!(request.dht_message_type, DhtMessageType::SafRequestMessages);
            });
        });
    }

    #[test]
    fn send_join_request() {
        runtime::test_async(|rt| {
            let node_identity = make_node_identity();
            let (out_tx, mut out_rx) = mpsc::channel(1);
            let (actor_tx, actor_rx) = mpsc::channel(1);
            let mut requester = DhtRequester::new(actor_tx);
            let outbound_requester = OutboundMessageRequester::new(out_tx);
            let shutdown = Shutdown::new();
            let actor = DhtActor::new(
                DhtConfig {
                    enable_auto_join: false,
                    enable_auto_stored_message_request: false,
                    ..Default::default()
                },
                node_identity,
                outbound_requester,
                actor_rx,
                shutdown.to_signal(),
            );

            rt.spawn(actor.start());

            rt.block_on(async move {
                requester.send_join().await.unwrap();
                let request = unwrap_oms_send_msg!(out_rx.next().await.unwrap());
                assert_eq!(request.dht_message_type, DhtMessageType::Join);
            });
        });
    }

    #[test]
    fn send_discover_request() {
        runtime::test_async(|rt| {
            let node_identity = make_node_identity();
            let (out_tx, mut out_rx) = mpsc::channel(1);
            let (actor_tx, actor_rx) = mpsc::channel(1);
            let mut requester = DhtRequester::new(actor_tx);
            let outbound_requester = OutboundMessageRequester::new(out_tx);
            let shutdown = Shutdown::new();
            let actor = DhtActor::new(
                DhtConfig {
                    enable_auto_join: false,
                    enable_auto_stored_message_request: false,
                    ..Default::default()
                },
                node_identity,
                outbound_requester,
                actor_rx,
                shutdown.to_signal(),
            );

            rt.spawn(actor.start());

            rt.block_on(async move {
                requester
                    .send_discover(CommsPublicKey::default(), None, NodeDestination::Unknown)
                    .await
                    .unwrap();
                let request = unwrap_oms_send_msg!(out_rx.next().await.unwrap());
                assert_eq!(request.dht_message_type, DhtMessageType::Discover);
            });
        });
    }

    #[test]
    fn insert_message_signature() {
        runtime::test_async(|rt| {
            let node_identity = make_node_identity();
            let (out_tx, _) = mpsc::channel(1);
            let (actor_tx, actor_rx) = mpsc::channel(1);
            let mut requester = DhtRequester::new(actor_tx);
            let outbound_requester = OutboundMessageRequester::new(out_tx);
            let shutdown = Shutdown::new();
            let actor = DhtActor::new(
                DhtConfig {
                    enable_auto_join: false,
                    enable_auto_stored_message_request: false,
                    ..Default::default()
                },
                node_identity,
                outbound_requester,
                actor_rx,
                shutdown.to_signal(),
            );

            rt.spawn(actor.start());

            rt.block_on(async move {
                let signature = vec![1u8, 2, 3];
                let is_dup = requester.insert_message_signature(signature.clone()).await.unwrap();
                assert_eq!(is_dup, false);
                let is_dup = requester.insert_message_signature(signature).await.unwrap();
                assert_eq!(is_dup, true);
                let is_dup = requester.insert_message_signature(Vec::new()).await.unwrap();
                assert_eq!(is_dup, false);
            });
        });
    }
}