use openraft::error::{InstallSnapshotError, NetworkError, RPCError, Unreachable};
use openraft::network::{RPCOption, RaftNetwork, RaftNetworkFactory};
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse,
    VoteRequest, VoteResponse,
};
use openraft::BasicNode;
use tonic::transport::Channel;

use crate::raft::{NodeId, TypeConfig};

/// Proto types generated from cluster.proto.
pub mod proto {
    tonic::include_proto!("synaptica.cluster.v1");
}

use proto::cluster_service_client::ClusterServiceClient;
use proto::RaftMessage;

// Type aliases for error types
type RaftRPCError<E = openraft::error::Infallible> =
    RPCError<NodeId, BasicNode, openraft::error::RaftError<NodeId, E>>;

/// Network factory that creates gRPC connections to peer nodes.
#[derive(Clone)]
pub struct GrpcNetwork;

impl RaftNetworkFactory<TypeConfig> for GrpcNetwork {
    type Network = GrpcNetworkConnection;

    async fn new_client(&mut self, target: NodeId, node: &BasicNode) -> Self::Network {
        GrpcNetworkConnection {
            _target: target,
            target_addr: node.addr.clone(),
        }
    }
}

/// A gRPC connection to a single peer node for Raft RPCs.
pub struct GrpcNetworkConnection {
    _target: NodeId,
    target_addr: String,
}

impl GrpcNetworkConnection {
    async fn connect(&self) -> Result<ClusterServiceClient<Channel>, tonic::transport::Error> {
        let addr = if self.target_addr.starts_with("http") {
            self.target_addr.clone()
        } else {
            format!("http://{}", self.target_addr)
        };
        ClusterServiceClient::connect(addr).await
    }
}

impl RaftNetwork<TypeConfig> for GrpcNetworkConnection {
    async fn append_entries(
        &mut self,
        req: AppendEntriesRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<NodeId>, RaftRPCError> {
        let data =
            bincode::serialize(&req).map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

        let mut client = self
            .connect()
            .await
            .map_err(|e| RPCError::Unreachable(Unreachable::new(&e)))?;

        let resp = client
            .append_entries(tonic::Request::new(RaftMessage { data }))
            .await
            .map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

        let response: AppendEntriesResponse<NodeId> = bincode::deserialize(&resp.into_inner().data)
            .map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

        Ok(response)
    }

    async fn install_snapshot(
        &mut self,
        req: InstallSnapshotRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<InstallSnapshotResponse<NodeId>, RaftRPCError<InstallSnapshotError>> {
        let data =
            bincode::serialize(&req).map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

        let mut client = self
            .connect()
            .await
            .map_err(|e| RPCError::Unreachable(Unreachable::new(&e)))?;

        let resp = client
            .install_snapshot(tonic::Request::new(RaftMessage { data }))
            .await
            .map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

        let response: InstallSnapshotResponse<NodeId> =
            bincode::deserialize(&resp.into_inner().data)
                .map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

        Ok(response)
    }

    async fn vote(
        &mut self,
        req: VoteRequest<NodeId>,
        _option: RPCOption,
    ) -> Result<VoteResponse<NodeId>, RaftRPCError> {
        let data =
            bincode::serialize(&req).map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

        let mut client = self
            .connect()
            .await
            .map_err(|e| RPCError::Unreachable(Unreachable::new(&e)))?;

        let resp = client
            .vote(tonic::Request::new(RaftMessage { data }))
            .await
            .map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

        let response: VoteResponse<NodeId> = bincode::deserialize(&resp.into_inner().data)
            .map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

        Ok(response)
    }
}
