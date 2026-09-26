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
