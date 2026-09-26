use uuid::Uuid;

/// One object key per media row: `c/{container_id}/{media_id}/orig`.
/// Namespaced by container (not by uploader) so a container's objects sit
/// together — useful for future per-container bulk operations (delete on
/// container-delete, storage accounting audits) without a bucket-wide scan.
pub fn media_object_key(container_id: Uuid, media_id: Uuid) -> String {
    format!("c/{container_id}/{media_id}/orig")
}
