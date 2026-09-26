use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Free-tier default per-container caps (see backend/AGENTS.md "Design
/// decisions already made" — R2's 10 GB free / zero-egress tier shapes
/// this). Owners can't change these yet (no PATCH /containers endpoint);
/// these are just the seeded defaults new containers get.
pub const DEFAULT_STORAGE_LIMIT_BYTES: i64 = 2 * 1024 * 1024 * 1024; // 2 GiB
pub const DEFAULT_MEDIA_LIMIT: i32 = 2000;

/// A single uploaded file's largest allowed size, checked before a
/// presigned upload URL is even issued (independent of the container's
/// overall quota).
pub const MAX_UPLOAD_SIZE_BYTES: i64 = 512 * 1024 * 1024; // 512 MiB

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaStatus {
    Pending,
    Ready,
}

impl MediaStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            MediaStatus::Pending => "pending",
            MediaStatus::Ready => "ready",
        }
    }
}

impl FromStr for MediaStatus {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(MediaStatus::Pending),
            "ready" => Ok(MediaStatus::Ready),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Media {
    pub id: Uuid,
    pub container_id: Uuid,
    pub uploader_id: String,
    pub object_key: String,
    pub filename: String,
    pub caption: Option<String>,
    pub content_type: String,
    pub size_bytes: i64,
    pub status: MediaStatus,
}

// Compile-time sanity check on the constants themselves: a single
// upload shouldn't be allowed to exceed a brand-new container's entire
// storage quota outright. A `#[test]` would just be asserting on two
// `const`s, which clippy (rightly) flags as vacuous — this is checked
// once, at compile time, instead.
const _: () = assert!(MAX_UPLOAD_SIZE_BYTES < DEFAULT_STORAGE_LIMIT_BYTES);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_status_from_str_parses_known_values() {
        assert_eq!("pending".parse::<MediaStatus>(), Ok(MediaStatus::Pending));
        assert_eq!("ready".parse::<MediaStatus>(), Ok(MediaStatus::Ready));
    }

    #[test]
    fn media_status_from_str_rejects_unknown_values() {
        assert!("".parse::<MediaStatus>().is_err());
        assert!("Ready".parse::<MediaStatus>().is_err()); // case-sensitive
        assert!("uploading".parse::<MediaStatus>().is_err());
    }

    #[test]
    fn media_status_as_str_round_trips_through_from_str() {
        for status in [MediaStatus::Pending, MediaStatus::Ready] {
            assert_eq!(status.as_str().parse::<MediaStatus>(), Ok(status));
        }
    }
}
