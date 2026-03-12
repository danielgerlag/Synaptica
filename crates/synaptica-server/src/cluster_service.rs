use std::sync::Arc;

use tonic::{Request, Response, Status};

use synaptica_cluster::raft::{NodeId, RaftRequest, SynapticaRaft, TypeConfig};
use synaptica_cluster::state_machine::StateMachineApplier;

pub mod proto {
    tonic::include_proto!("synaptica.cluster.v1");
}

use proto::cluster_service_server::ClusterService;
use proto::{
    AddLearnerRequest, AddLearnerResponse, ChangeMembershipRequest, ChangeMembershipResponse,
    ForwardWriteRequest, ForwardWriteResponse, RaftMessage,
};

/// Implements the ClusterService gRPC server for inter-node Raft communication.
pub struct ClusterServiceImpl {
    pub raft: SynapticaRaft,
    pub applier: Arc<StateMachineApplier>,
}

#[tonic::async_trait]
impl ClusterService for ClusterServiceImpl {
    async fn vote(
        &self,
        request: Request<RaftMessage>,
    ) -> Result<Response<RaftMessage>, Status> {
        let data = request.into_inner().data;
        let req: openraft::raft::VoteRequest<NodeId> = bincode::deserialize(&data)
            .map_err(|e| Status::invalid_argument(format!("deserialize vote request: {}", e)))?;

        let resp = self
            .raft
            .vote(req)
            .await
            .map_err(|e| Status::internal(format!("raft vote: {}", e)))?;

        let resp_data = bincode::serialize(&resp)
            .map_err(|e| Status::internal(format!("serialize vote response: {}", e)))?;

        Ok(Response::new(RaftMessage { data: resp_data }))
    }

    async fn append_entries(
        &self,
        request: Request<RaftMessage>,
    ) -> Result<Response<RaftMessage>, Status> {
        let data = request.into_inner().data;
        let req: openraft::raft::AppendEntriesRequest<TypeConfig> =
            bincode::deserialize(&data).map_err(|e| {
                Status::invalid_argument(format!(
                    "deserialize append_entries request: {}",
                    e
                ))
            })?;

        let resp = self
            .raft
            .append_entries(req)
            .await
            .map_err(|e| Status::internal(format!("raft append_entries: {}", e)))?;

        let resp_data = bincode::serialize(&resp)
            .map_err(|e| Status::internal(format!("serialize append_entries response: {}", e)))?;

        Ok(Response::new(RaftMessage { data: resp_data }))
    }

    async fn install_snapshot(
        &self,
        request: Request<RaftMessage>,
    ) -> Result<Response<RaftMessage>, Status> {
        let data = request.into_inner().data;
        let req: openraft::raft::InstallSnapshotRequest<TypeConfig> =
            bincode::deserialize(&data).map_err(|e| {
                Status::invalid_argument(format!(
                    "deserialize install_snapshot request: {}",
                    e
                ))
            })?;

        let resp = self
            .raft
            .install_snapshot(req)
            .await
            .map_err(|e| Status::internal(format!("raft install_snapshot: {}", e)))?;

        let resp_data = bincode::serialize(&resp).map_err(|e| {
            Status::internal(format!("serialize install_snapshot response: {}", e))
        })?;

        Ok(Response::new(RaftMessage { data: resp_data }))
    }

    async fn forward_write(
        &self,
        request: Request<ForwardWriteRequest>,
    ) -> Result<Response<ForwardWriteResponse>, Status> {
        let req = request.into_inner();
        let raft_req = RaftRequest::WriteQuery {
            query: req.query,
            graph_name: req.graph_name,
        };

        match self.raft.client_write(raft_req.clone()).await {
            Ok(_resp) => {
                let result = self.applier.apply(&raft_req);
                Ok(Response::new(ForwardWriteResponse {
                    success: result.success,
                    error: result.error,
                    rows_affected: result.rows_affected,
                }))
            }
            Err(e) => Ok(Response::new(ForwardWriteResponse {
                success: false,
                error: Some(format!("raft write: {}", e)),
                rows_affected: 0,
            })),
        }
    }

    async fn add_learner(
        &self,
        request: Request<AddLearnerRequest>,
    ) -> Result<Response<AddLearnerResponse>, Status> {
        let req = request.into_inner();
        let node = openraft::BasicNode {
            addr: req.address,
        };

        match self.raft.add_learner(req.node_id, node, true).await {
            Ok(_) => Ok(Response::new(AddLearnerResponse {
                success: true,
                error: None,
            })),
            Err(e) => Ok(Response::new(AddLearnerResponse {
                success: false,
                error: Some(format!("{}", e)),
            })),
        }
    }

    async fn change_membership(
        &self,
        request: Request<ChangeMembershipRequest>,
    ) -> Result<Response<ChangeMembershipResponse>, Status> {
        let req = request.into_inner();
        let voters: std::collections::BTreeSet<NodeId> =
            req.voter_ids.into_iter().collect();

        match self.raft.change_membership(voters, false).await {
            Ok(_) => Ok(Response::new(ChangeMembershipResponse {
                success: true,
                error: None,
            })),
            Err(e) => Ok(Response::new(ChangeMembershipResponse {
                success: false,
                error: Some(format!("{}", e)),
            })),
        }
    }
}
