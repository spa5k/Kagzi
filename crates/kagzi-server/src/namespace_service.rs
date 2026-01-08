use kagzi_proto::kagzi::namespace_service_server::NamespaceService;
use kagzi_proto::kagzi::{
    CreateNamespaceRequest, CreateNamespaceResponse, DeleteNamespaceRequest,
    DeleteNamespaceResponse, DeletionMode, GetNamespaceRequest, GetNamespaceResponse,
    ListNamespacesRequest, ListNamespacesResponse, Namespace as ProtoNamespace, ResourceCount,
    UpdateNamespaceRequest, UpdateNamespaceResponse,
};
use kagzi_store::postgres::PgStore;
use kagzi_store::repository::NamespaceRepository;
use tonic::{Request, Response, Status};
use tracing::instrument;

use crate::helpers::{
    invalid_argument_error, map_store_error, normalize_page_size, not_found_error,
    require_non_empty,
};
use crate::proto_convert::timestamp_from;

#[derive(Clone)]
pub struct NamespaceServiceImpl {
    store: PgStore,
}

impl NamespaceServiceImpl {
    pub fn new(store: PgStore) -> Self {
        Self { store }
    }
}

#[tonic::async_trait]
impl NamespaceService for NamespaceServiceImpl {
    #[instrument(skip(self, request))]
    async fn create_namespace(
        &self,
        request: Request<CreateNamespaceRequest>,
    ) -> Result<Response<CreateNamespaceResponse>, Status> {
        let req = request.into_inner();

        let namespace_id = require_non_empty(req.namespace_id, "namespace_id")?;
        let display_name = require_non_empty(req.display_name, "display_name")?;

        let params = kagzi_store::models::CreateNamespace {
            namespace_id,
            display_name,
            description: req.description,
        };

        let namespace = self
            .store
            .namespaces()
            .create(params)
            .await
            .map_err(map_store_error)?;

        Ok(Response::new(CreateNamespaceResponse {
            namespace: Some(namespace_to_proto(namespace)?),
        }))
    }

    #[instrument(skip(self, request))]
    async fn get_namespace(
        &self,
        request: Request<GetNamespaceRequest>,
    ) -> Result<Response<GetNamespaceResponse>, Status> {
        let req = request.into_inner();
        let namespace_id = require_non_empty(req.namespace_id, "namespace_id")?;

        let namespace = self
            .store
            .namespaces()
            .find_by_id(&namespace_id, false)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| {
                not_found_error(
                    format!("Namespace '{}' not found", namespace_id),
                    "namespace",
                    &namespace_id,
                )
            })?;

        Ok(Response::new(GetNamespaceResponse {
            namespace: Some(namespace_to_proto(namespace)?),
        }))
    }

    #[instrument(skip(self, request))]
    async fn list_namespaces(
        &self,
        request: Request<ListNamespacesRequest>,
    ) -> Result<Response<ListNamespacesResponse>, Status> {
        let req = request.into_inner();

        let page_req = req
            .page
            .ok_or_else(|| invalid_argument_error("page is required"))?;
        let page_size = normalize_page_size(page_req.page_size, 20, 100);
        let cursor = if page_req.page_token.is_empty() {
            None
        } else {
            Some(page_req.page_token)
        };

        let result = self
            .store
            .namespaces()
            .list(page_size, cursor, req.include_deleted)
            .await
            .map_err(map_store_error)?;

        let namespaces: Result<Vec<ProtoNamespace>, Status> =
            result.items.into_iter().map(namespace_to_proto).collect();

        let next_page_token = result.next_cursor.unwrap_or_default();

        Ok(Response::new(ListNamespacesResponse {
            namespaces: namespaces?,
            page: Some(kagzi_proto::kagzi::PageInfo {
                next_page_token,
                has_more: result.has_more,
                total_count: 0, // Not computed for efficiency
            }),
        }))
    }

    #[instrument(skip(self, request))]
    async fn update_namespace(
        &self,
        request: Request<UpdateNamespaceRequest>,
    ) -> Result<Response<UpdateNamespaceResponse>, Status> {
        let req = request.into_inner();
        let namespace_id = require_non_empty(req.namespace_id, "namespace_id")?;

        let namespace = self
            .store
            .namespaces()
            .update(&namespace_id, req.display_name, req.description)
            .await
            .map_err(map_store_error)?;

        Ok(Response::new(UpdateNamespaceResponse {
            namespace: Some(namespace_to_proto(namespace)?),
        }))
    }

    #[instrument(skip(self, request))]
    async fn delete_namespace(
        &self,
        request: Request<DeleteNamespaceRequest>,
    ) -> Result<Response<DeleteNamespaceResponse>, Status> {
        let req = request.into_inner();
        let namespace_id = require_non_empty(req.namespace_id, "namespace_id")?;

        let mode = DeletionMode::try_from(req.mode).unwrap_or(DeletionMode::FailIfResources);

        match mode {
            DeletionMode::FailIfResources | DeletionMode::Unspecified => {
                // Check for resources first
                let resources = self
                    .store
                    .namespaces()
                    .count_resources(&namespace_id)
                    .await
                    .map_err(map_store_error)?;

                if resources.has_resources() {
                    return Ok(Response::new(DeleteNamespaceResponse {
                        deleted: false,
                        message: format!(
                            "Cannot delete namespace '{}': {} workflows, {} workers, {} schedules exist",
                            namespace_id,
                            resources.workflows,
                            resources.workers,
                            resources.schedules
                        ),
                        error: Some(kagzi_proto::kagzi::ErrorDetail {
                            code: kagzi_proto::kagzi::ErrorCode::PreconditionFailed as i32,
                            message: "Namespace has active resources".to_string(),
                            non_retryable: true,
                            retry_after_ms: 0,
                            subject: "namespace".to_string(),
                            subject_id: namespace_id.clone(),
                            metadata: Default::default(),
                        }),
                        resources: Some(ResourceCount {
                            workflows: resources.workflows,
                            workers: resources.workers,
                            schedules: resources.schedules,
                        }),
                    }));
                }

                // No resources, proceed with hard delete
                let deleted = self
                    .store
                    .namespaces()
                    .delete(&namespace_id)
                    .await
                    .map_err(map_store_error)?;

                Ok(Response::new(DeleteNamespaceResponse {
                    deleted,
                    message: if deleted {
                        format!("Namespace '{}' deleted successfully", namespace_id)
                    } else {
                        format!("Namespace '{}' not found", namespace_id)
                    },
                    error: None,
                    resources: None,
                }))
            }
            DeletionMode::SoftDelete => {
                let deleted = self
                    .store
                    .namespaces()
                    .soft_delete(&namespace_id)
                    .await
                    .map_err(map_store_error)?;

                Ok(Response::new(DeleteNamespaceResponse {
                    deleted,
                    message: if deleted {
                        format!("Namespace '{}' soft-deleted successfully", namespace_id)
                    } else {
                        format!("Namespace '{}' not found", namespace_id)
                    },
                    error: None,
                    resources: None,
                }))
            }
            DeletionMode::Cascade => {
                // CASCADE deletion: delete all resources first, then namespace
                // Note: This is dangerous and should be used with caution

                // For now, return an error as CASCADE requires careful implementation
                // to handle foreign key constraints and transaction management
                return Err(Status::unimplemented(
                    "CASCADE deletion mode is not yet implemented. Use SOFT_DELETE or manually delete resources first.",
                ));
            }
        }
    }
}

/// Convert store Namespace model to proto Namespace
fn namespace_to_proto(namespace: kagzi_store::models::Namespace) -> Result<ProtoNamespace, Status> {
    Ok(ProtoNamespace {
        namespace_id: namespace.namespace_id,
        display_name: namespace.display_name,
        description: namespace.description.unwrap_or_default(),
        created_at: Some(timestamp_from(namespace.created_at)),
        updated_at: Some(timestamp_from(namespace.updated_at)),
    })
}
