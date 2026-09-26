use uuid::Uuid;

/// One object key per media row: `c/{container_id}/{media_id}/orig`.
/// Namespaced by container (not by uploader) so a container's objects sit
/// together — useful for future per-container bulk operations (delete on
/// container-delete, storage accounting audits) without a bucket-wide scan.
pub fn media_object_key(container_id: Uuid, media_id: Uuid) -> String {
    format!("c/{container_id}/{media_id}/orig")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_has_the_expected_shape() {
        let container_id = Uuid::nil();
        let media_id = Uuid::max();

        let key = media_object_key(container_id, media_id);

        assert_eq!(
            key,
            "c/00000000-0000-0000-0000-000000000000/ffffffff-ffff-ffff-ffff-ffffffffffff/orig"
        );
    }

    #[test]
    fn different_media_in_the_same_container_get_different_keys() {
        let container_id = Uuid::nil();
        let key_a = media_object_key(container_id, Uuid::from_u128(1));
        let key_b = media_object_key(container_id, Uuid::from_u128(2));

        assert_ne!(key_a, key_b);
        // Both still namespaced under the same container prefix.
        assert!(key_a.starts_with(&format!("c/{container_id}/")));
        assert!(key_b.starts_with(&format!("c/{container_id}/")));
    }
}
