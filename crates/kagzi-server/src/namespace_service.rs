use kagzi_proto::kagzi::namespace_service_server::NamespaceService;
use kagzi_proto::kagzi::{
    CreateNamespaceRequest, CreateNamespaceResponse, DisableNamespaceRequest,
    DisableNamespaceResponse, EnableNamespaceRequest, EnableNamespaceResponse, GetNamespaceRequest,
    GetNamespaceResponse, ListNamespacesRequest, ListNamespacesResponse,
    Namespace as ProtoNamespace, UpdateNamespaceRequest, UpdateNamespaceResponse,
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

        let namespace = require_non_empty(req.namespace, "namespace")?;
        let display_name = require_non_empty(req.display_name, "display_name")?;

        let params = kagzi_store::models::CreateNamespace {
            namespace,
            display_name,
            description: req.description,
        };

        let ns = self
            .store
            .namespaces()
            .create(params)
            .await
            .map_err(map_store_error)?;

        Ok(Response::new(CreateNamespaceResponse {
            namespace: Some(namespace_to_proto(ns)?),
        }))
    }

    #[instrument(skip(self, request))]
    async fn get_namespace(
        &self,
        request: Request<GetNamespaceRequest>,
    ) -> Result<Response<GetNamespaceResponse>, Status> {
        let req = request.into_inner();
        let namespace = require_non_empty(req.namespace, "namespace")?;

        let ns = self
            .store
            .namespaces()
            .find_by_id(&namespace)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| {
                not_found_error(
                    format!("Namespace '{}' not found", namespace),
                    "namespace",
                    &namespace,
                )
            })?;

        Ok(Response::new(GetNamespaceResponse {
            namespace: Some(namespace_to_proto(ns)?),
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
            .list(page_size, cursor)
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
        let namespace = require_non_empty(req.namespace, "namespace")?;

        let params = kagzi_store::models::UpdateNamespace {
            display_name: req.display_name,
            description: req.description,
        };

        let ns = self
            .store
            .namespaces()
            .update(&namespace, params)
            .await
            .map_err(map_store_error)?;

        Ok(Response::new(UpdateNamespaceResponse {
            namespace: Some(namespace_to_proto(ns)?),
        }))
    }

    #[instrument(skip(self, request))]
    async fn enable_namespace(
        &self,
        request: Request<EnableNamespaceRequest>,
    ) -> Result<Response<EnableNamespaceResponse>, Status> {
        let req = request.into_inner();
        let namespace = require_non_empty(req.namespace, "namespace")?;

        self.store
            .namespaces()
            .enable(&namespace)
            .await
            .map_err(map_store_error)?;

        let ns = self
            .store
            .namespaces()
            .find_by_id(&namespace)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| not_found_error("Namespace not found", "namespace", &namespace))?;

        Ok(Response::new(EnableNamespaceResponse {
            namespace: Some(namespace_to_proto(ns)?),
        }))
    }

    #[instrument(skip(self, request))]
    async fn disable_namespace(
        &self,
        request: Request<DisableNamespaceRequest>,
    ) -> Result<Response<DisableNamespaceResponse>, Status> {
        let req = request.into_inner();
        let namespace = require_non_empty(req.namespace, "namespace")?;

        self.store
            .namespaces()
            .disable(&namespace)
            .await
            .map_err(map_store_error)?;

        let ns = self
            .store
            .namespaces()
            .find_by_id(&namespace)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| not_found_error("Namespace not found", "namespace", &namespace))?;

        Ok(Response::new(DisableNamespaceResponse {
            namespace: Some(namespace_to_proto(ns)?),
        }))
    }
}

/// Convert store Namespace model to proto Namespace
fn namespace_to_proto(namespace: kagzi_store::models::Namespace) -> Result<ProtoNamespace, Status> {
    Ok(ProtoNamespace {
        id: namespace.id.to_string(),
        namespace: namespace.namespace,
        display_name: namespace.display_name,
        description: namespace.description.unwrap_or_default(),
        enabled: namespace.enabled,
        created_at: Some(timestamp_from(namespace.created_at)),
        updated_at: Some(timestamp_from(namespace.updated_at)),
    })
}
