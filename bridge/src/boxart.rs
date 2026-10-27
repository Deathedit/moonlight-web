//! On-disk box art cache: `<data_dir>/boxart/<host>/<app_id>.img`.
//! Sunshine derives app ids from the app's name and image, so a changed image gets a
//! new id and stale entries are simply never requested again.

use std::path::PathBuf;

pub struct BoxArtCache {
    dir: PathBuf,
}

impl BoxArtCache {
    pub fn new(data_dir: &std::path::Path) -> Self {
        Self { dir: data_dir.join("boxart") }
    }

    pub async fn get(&self, host_id: &str, app_id: u32) -> Option<Vec<u8>> {
        tokio::fs::read(self.path(host_id, app_id)).await.ok()
    }

    pub async fn put(&self, host_id: &str, app_id: u32, data: &[u8]) -> std::io::Result<()> {
        let path = self.path(host_id, app_id);
        tokio::fs::create_dir_all(path.parent().unwrap()).await?;
        crate::util::write_atomic(&path, data).await
    }

    pub async fn remove_host(&self, host_id: &str) {
        let _ = tokio::fs::remove_dir_all(self.dir.join(host_dir(host_id))).await;
    }

    fn path(&self, host_id: &str, app_id: u32) -> PathBuf {
        self.dir.join(host_dir(host_id)).join(format!("{app_id}.img"))
    }
}

/// Host ids come from the host itself, so never trust them as a path component.
fn host_dir(host_id: &str) -> String {
    let safe = !host_id.is_empty() && host_id.len() <= 64 && host_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if safe {
        host_id.to_string()
    } else {
        hex::encode(&openssl::sha::sha256(host_id.as_bytes())[..16])
    }
}

/// Sniff the image type for the Content-Type header.
pub fn content_type(data: &[u8]) -> &'static str {
    match data {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [0xFF, 0xD8, 0xFF, ..] => "image/jpeg",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_dir_rejects_traversal() {
        assert_eq!(host_dir("C18D8A40-06AD-CA0D-DD18-DA58B543DBAB"), "C18D8A40-06AD-CA0D-DD18-DA58B543DBAB");
        for evil in ["../../etc", "a/b", "", ".."] {
            let d = host_dir(evil);
            assert!(d.chars().all(|c| c.is_ascii_hexdigit()) && d.len() == 32, "{evil:?} -> {d}");
        }
    }

    #[test]
    fn sniffs_types() {
        assert_eq!(content_type(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A]), "image/png");
        assert_eq!(content_type(&[0xFF, 0xD8, 0xFF, 0xE0]), "image/jpeg");
        assert_eq!(content_type(b"nope"), "application/octet-stream");
    }
}
