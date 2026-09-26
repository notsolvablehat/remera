use std::time::Duration;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use domain::{DomainError, MAX_UPLOAD_SIZE_BYTES, MediaStatus};
use serde::{Deserialize, Serialize};
use storage::media_repo::{self, MediaRecord, MediaRepoError};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;
use validator::Validate;

use crate::{
    extractors::container_access::{ContainerAccess, ContainerViewAccess, Editor},
    state::AppState,
};

const UPLOAD_URL_TTL: Duration = Duration::from_secs(15 * 60);
const DOWNLOAD_URL_TTL: Duration = Duration::from_secs(5 * 60);
const DEFAULT_PAGE_LIMIT: i64 = 50;
const MAX_PAGE_LIMIT: i64 = 100;

#[derive(Deserialize, Validate, ToSchema)]
pub struct CreateUploadRequest {
    #[validate(length(min = 1, max = 255))]
    pub filename: String,
    #[validate(length(min = 1, max = 255))]
    pub content_type: String,
    #[validate(range(min = 1, max = "MAX_UPLOAD_SIZE_BYTES"))]
    pub size_bytes: i64,
}

#[derive(Serialize, ToSchema)]
pub struct CreateUploadResponse {
    pub media_id: Uuid,
    pub upload_url: String,
    pub expires_in_seconds: u64,
}

#[derive(Serialize, ToSchema)]
pub struct MediaDto {
    pub id: Uuid,
    pub uploader_id: String,
    pub filename: String,
    pub caption: Option<String>,
    pub content_type: String,
    pub size_bytes: i64,
    pub status: String,
    pub created_at: String,
}

impl From<MediaRecord> for MediaDto {
    fn from(record: MediaRecord) -> Self {
        MediaDto {
            id: record.media.id,
            uploader_id: record.media.uploader_id,
            filename: record.media.filename,
            caption: record.media.caption,
            content_type: record.media.content_type,
            size_bytes: record.media.size_bytes,
            status: record.media.status.as_str().to_string(),
            created_at: record.created_at.to_rfc3339(),
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct MediaListResponse {
    pub media: Vec<MediaDto>,
    pub next_cursor: Option<Uuid>,
}

#[derive(Deserialize, ToSchema)]
pub struct ListMediaQuery {
    pub cursor: Option<Uuid>,
    pub limit: Option<i64>,
    #[serde(rename = "type")]
    pub media_type: Option<String>,
    pub uploader: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct DownloadUrlResponse {
    pub url: String,
    pub expires_in_seconds: u64,
}

#[derive(Deserialize, Validate, ToSchema)]
pub struct UpdateMediaRequest {
    #[validate(length(min = 1, max = 255))]
    pub filename: Option<String>,
    #[validate(length(max = 2000))]
    pub caption: Option<String>,
}

fn error_response(status: StatusCode, error: &str) -> axum::response::Response {
    (status, Json(serde_json::json!({"error": error}))).into_response()
}

fn db_error() -> axum::response::Response {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, "db_error")
}

fn media_repo_error_response(err: MediaRepoError) -> axum::response::Response {
    match err {
        MediaRepoError::Domain(DomainError::QuotaExceeded) => {
            error_response(StatusCode::CONFLICT, "container_quota_exceeded")
        }
        _ => db_error(),
    }
}

/// Not `ContainerAccess`'s job (that only checks role) — every media
/// handler that scopes by `{media_id}` also has to confirm the row
/// actually belongs to the container in the URL, not just that the
/// caller has a role somewhere in that container.
fn media_in_container(record: &MediaRecord, container_id: Uuid) -> bool {
    record.media.container_id == container_id
}

#[utoipa::path(
    post,
    path = "/containers/{container_id}/uploads",
    tag = "Media",
    params(("container_id" = Uuid, Path, description = "Container id")),
    request_body = CreateUploadRequest,
    responses(
        (status = 201, description = "Pending media row created, presigned upload URL issued", body = CreateUploadResponse),
        (status = 400, description = "Invalid filename/content-type/size"),
        (status = 403, description = "Caller lacks Edit access"),
        (status = 409, description = "Container storage/media quota exceeded"),
    )
)]
async fn create_upload(
    access: ContainerAccess<Editor>,
    State(state): State<AppState>,
    Path(container_id): Path<Uuid>,
    Json(body): Json<CreateUploadRequest>,
) -> axum::response::Response {
    if body.validate().is_err() {
        return error_response(StatusCode::BAD_REQUEST, "invalid_request");
    }

    let media_id = Uuid::now_v7();
    let object_key = r2::keys::media_object_key(container_id, media_id);

    let media = match media_repo::reserve_and_create_pending(
        &state.db,
        media_id,
        container_id,
        &access.user.id,
        &object_key,
        &body.filename,
        &body.content_type,
        body.size_bytes,
    )
    .await
    {
        Ok(media) => media,
        Err(err) => return media_repo_error_response(err),
    };

    match r2::presigned_put_url(
        &state.r2,
        &state.r2_bucket,
        &media.object_key,
        &body.content_type,
        UPLOAD_URL_TTL,
    )
    .await
    {
        Ok(upload_url) => (
            StatusCode::CREATED,
            Json(CreateUploadResponse {
                media_id: media.id,
                upload_url,
                expires_in_seconds: UPLOAD_URL_TTL.as_secs(),
            }),
        )
            .into_response(),
        Err(err) => {
            tracing::error!(?err, "failed to presign upload url");
            db_error()
        }
    }
}

#[utoipa::path(
    post,
    path = "/containers/{container_id}/uploads/{media_id}/complete",
    tag = "Media",
    params(
        ("container_id" = Uuid, Path, description = "Container id"),
        ("media_id" = Uuid, Path, description = "Media id"),
    ),
    responses(
        (status = 200, description = "Upload verified and marked ready", body = MediaDto),
        (status = 403, description = "Not the uploader"),
        (status = 404, description = "Pending upload not found"),
        (status = 409, description = "Object not found in storage yet, or size mismatch"),
    )
)]
async fn complete_upload(
    access: ContainerAccess<Editor>,
    State(state): State<AppState>,
    Path((container_id, media_id)): Path<(Uuid, Uuid)>,
) -> axum::response::Response {
    let record = match media_repo::get_by_id(&state.db, media_id).await {
        Ok(Some(record)) => record,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found"),
        Err(_) => return db_error(),
    };

    if !media_in_container(&record, container_id) {
        return error_response(StatusCode::NOT_FOUND, "not_found");
    }

    if record.media.uploader_id != access.user.id {
        return error_response(StatusCode::FORBIDDEN, "forbidden");
    }

    if record.media.status == MediaStatus::Ready {
        return Json(MediaDto::from(MediaRecord {
            media: record.media.clone(),
            created_at: record.created_at,
        }))
        .into_response();
    }

    let verified_size =
        match r2::verify_uploaded_object(&state.r2, &state.r2_bucket, &record.media.object_key)
            .await
        {
            Ok(size) => size,
            Err(err) => {
                tracing::error!(?err, "failed to head uploaded object");
                return db_error();
            }
        };

    match verified_size {
        Some(size) if size == record.media.size_bytes => {}
        Some(_) => return error_response(StatusCode::CONFLICT, "size_mismatch"),
        None => return error_response(StatusCode::CONFLICT, "not_uploaded_yet"),
    }

    match media_repo::mark_ready(&state.db, media_id).await {
        Ok(_) => {}
        Err(_) => return db_error(),
    }

    match media_repo::get_by_id(&state.db, media_id).await {
        Ok(Some(record)) => Json(MediaDto::from(record)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found"),
        Err(_) => db_error(),
    }
}

#[utoipa::path(
    delete,
    path = "/containers/{container_id}/uploads/{media_id}",
    tag = "Media",
    params(
        ("container_id" = Uuid, Path, description = "Container id"),
        ("media_id" = Uuid, Path, description = "Media id"),
    ),
    responses(
        (status = 204, description = "Pending upload aborted, quota released"),
        (status = 403, description = "Not the uploader"),
        (status = 404, description = "Pending upload not found"),
    )
)]
async fn abort_upload(
    access: ContainerAccess<Editor>,
    State(state): State<AppState>,
    Path((container_id, media_id)): Path<(Uuid, Uuid)>,
) -> axum::response::Response {
    let record = match media_repo::get_by_id(&state.db, media_id).await {
        Ok(Some(record)) => record,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found"),
        Err(_) => return db_error(),
    };

    if !media_in_container(&record, container_id) {
        return error_response(StatusCode::NOT_FOUND, "not_found");
    }

    if record.media.uploader_id != access.user.id {
        return error_response(StatusCode::FORBIDDEN, "forbidden");
    }

    match media_repo::abort_pending(&state.db, media_id).await {
        Ok(Some(media)) => {
            if let Err(err) =
                r2::delete_object(&state.r2, &state.r2_bucket, &media.object_key).await
            {
                // Quota is already released and the DB row is gone —
                // an orphaned partial object in R2 is a cleanup-job
                // concern, not something worth failing this request over.
                tracing::warn!(?err, object_key = %media.object_key, "failed to delete aborted upload object");
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found"),
        Err(_) => db_error(),
    }
}

#[utoipa::path(
    get,
    path = "/containers/{container_id}/media",
    tag = "Media",
    params(
        ("container_id" = Uuid, Path, description = "Container id"),
        ("cursor" = Option<Uuid>, Query, description = "Keyset cursor: id of the last item from the previous page"),
        ("limit" = Option<i64>, Query, description = "Page size, capped at 100"),
        ("type" = Option<String>, Query, description = "Filter by content-type prefix: image or video"),
        ("uploader" = Option<String>, Query, description = "Filter by uploader user id"),
        ("share_token" = Option<String>, Query, description = "Anonymous View access via a container's share link (see GET /containers/{cid}/share-link)"),
    ),
    responses(
        (status = 200, description = "Ready media in the container", body = MediaListResponse),
        (status = 403, description = "Not a member and no valid share token"),
        (status = 404, description = "Container not found"),
        (status = 423, description = "Container is locked"),
    )
)]
async fn list_media(
    _access: ContainerViewAccess,
    State(state): State<AppState>,
    Path(container_id): Path<Uuid>,
    Query(query): Query<ListMediaQuery>,
) -> axum::response::Response {
    let limit = query
        .limit
        .unwrap_or(DEFAULT_PAGE_LIMIT)
        .clamp(1, MAX_PAGE_LIMIT);

    match media_repo::list_for_container(
        &state.db,
        container_id,
        query.cursor,
        limit,
        query.media_type.as_deref(),
        query.uploader.as_deref(),
    )
    .await
    {
        Ok(records) => {
            let next_cursor = if records.len() as i64 == limit {
                records.last().map(|r| r.media.id)
            } else {
                None
            };

            Json(MediaListResponse {
                media: records.into_iter().map(MediaDto::from).collect(),
                next_cursor,
            })
            .into_response()
        }
        Err(_) => db_error(),
    }
}

#[utoipa::path(
    get,
    path = "/containers/{container_id}/media/{media_id}",
    tag = "Media",
    params(
        ("container_id" = Uuid, Path, description = "Container id"),
        ("media_id" = Uuid, Path, description = "Media id"),
        ("share_token" = Option<String>, Query, description = "Anonymous View access via a container's share link"),
    ),
    responses(
        (status = 200, description = "Media metadata", body = MediaDto),
        (status = 403, description = "Not a member and no valid share token"),
        (status = 404, description = "Not found"),
        (status = 423, description = "Container is locked"),
    )
)]
async fn get_media(
    _access: ContainerViewAccess,
    State(state): State<AppState>,
    Path((container_id, media_id)): Path<(Uuid, Uuid)>,
) -> axum::response::Response {
    match media_repo::get_by_id(&state.db, media_id).await {
        Ok(Some(record))
            if record.media.container_id == container_id
                && record.media.status == MediaStatus::Ready =>
        {
            Json(MediaDto::from(record)).into_response()
        }
        Ok(_) => error_response(StatusCode::NOT_FOUND, "not_found"),
        Err(_) => db_error(),
    }
}

#[utoipa::path(
    get,
    path = "/containers/{container_id}/media/{media_id}/download",
    tag = "Media",
    params(
        ("container_id" = Uuid, Path, description = "Container id"),
        ("media_id" = Uuid, Path, description = "Media id"),
        ("share_token" = Option<String>, Query, description = "Anonymous View access via a container's share link"),
    ),
    responses(
        (status = 200, description = "Presigned download URL", body = DownloadUrlResponse),
        (status = 403, description = "Not a member and no valid share token"),
        (status = 404, description = "Not found"),
        (status = 423, description = "Container is locked"),
    )
)]
async fn download_media(
    _access: ContainerViewAccess,
    State(state): State<AppState>,
    Path((container_id, media_id)): Path<(Uuid, Uuid)>,
) -> axum::response::Response {
    let record = match media_repo::get_by_id(&state.db, media_id).await {
        Ok(Some(record))
            if record.media.container_id == container_id
                && record.media.status == MediaStatus::Ready =>
        {
            record
        }
        Ok(_) => return error_response(StatusCode::NOT_FOUND, "not_found"),
        Err(_) => return db_error(),
    };

    match r2::presigned_get_url(
        &state.r2,
        &state.r2_bucket,
        &record.media.object_key,
        &record.media.filename,
        DOWNLOAD_URL_TTL,
    )
    .await
    {
        Ok(url) => Json(DownloadUrlResponse {
            url,
            expires_in_seconds: DOWNLOAD_URL_TTL.as_secs(),
        })
        .into_response(),
        Err(err) => {
            tracing::error!(?err, "failed to presign download url");
            db_error()
        }
    }
}

#[utoipa::path(
    patch,
    path = "/containers/{container_id}/media/{media_id}",
    tag = "Media",
    params(
        ("container_id" = Uuid, Path, description = "Container id"),
        ("media_id" = Uuid, Path, description = "Media id"),
    ),
    request_body = UpdateMediaRequest,
    responses(
        (status = 200, description = "Updated", body = MediaDto),
        (status = 400, description = "Invalid request, or no fields to update"),
        (status = 403, description = "Not the uploader or container owner"),
        (status = 404, description = "Not found"),
    )
)]
async fn update_media(
    access: ContainerAccess<Editor>,
    State(state): State<AppState>,
    Path((container_id, media_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<UpdateMediaRequest>,
) -> axum::response::Response {
    if body.validate().is_err() || (body.filename.is_none() && body.caption.is_none()) {
        return error_response(StatusCode::BAD_REQUEST, "invalid_request");
    }

    let record = match media_repo::get_by_id(&state.db, media_id).await {
        Ok(Some(record)) => record,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found"),
        Err(_) => return db_error(),
    };

    if !media_in_container(&record, container_id) {
        return error_response(StatusCode::NOT_FOUND, "not_found");
    }

    if access.role != domain::Role::Owner && record.media.uploader_id != access.user.id {
        return error_response(StatusCode::FORBIDDEN, "forbidden");
    }

    match media_repo::update_metadata(
        &state.db,
        media_id,
        body.filename.as_deref(),
        body.caption.as_deref(),
    )
    .await
    {
        Ok(Some(record)) => Json(MediaDto::from(record)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found"),
        Err(_) => db_error(),
    }
}

#[utoipa::path(
    delete,
    path = "/containers/{container_id}/media/{media_id}",
    tag = "Media",
    params(
        ("container_id" = Uuid, Path, description = "Container id"),
        ("media_id" = Uuid, Path, description = "Media id"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 403, description = "Not the uploader or container owner"),
        (status = 404, description = "Not found"),
    )
)]
async fn delete_media(
    access: ContainerAccess<Editor>,
    State(state): State<AppState>,
    Path((container_id, media_id)): Path<(Uuid, Uuid)>,
) -> axum::response::Response {
    let record = match media_repo::get_by_id(&state.db, media_id).await {
        Ok(Some(record)) => record,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found"),
        Err(_) => return db_error(),
    };

    if !media_in_container(&record, container_id) {
        return error_response(StatusCode::NOT_FOUND, "not_found");
    }

    if access.role != domain::Role::Owner && record.media.uploader_id != access.user.id {
        return error_response(StatusCode::FORBIDDEN, "forbidden");
    }

    match media_repo::delete_ready(&state.db, media_id).await {
        Ok(Some(media)) => {
            if let Err(err) =
                r2::delete_object(&state.r2, &state.r2_bucket, &media.object_key).await
            {
                tracing::warn!(?err, object_key = %media.object_key, "failed to delete media object");
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found"),
        Err(_) => db_error(),
    }
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(create_upload))
        .routes(routes!(complete_upload))
        .routes(routes!(abort_upload))
        .routes(routes!(list_media))
        .routes(routes!(get_media))
        .routes(routes!(download_media))
        .routes(routes!(update_media))
        .routes(routes!(delete_media))
}
