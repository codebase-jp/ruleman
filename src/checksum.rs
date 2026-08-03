//! Hashing files, and the algorithms a `checksum` rule can name.

use clap::ValueEnum;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ChecksumAlgorithm {
    #[default]
    Sha256,
}

impl ChecksumAlgorithm {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ChecksumAlgorithm::Sha256 => "sha256",
        }
    }

    /// Length of this algorithm's digest written as hex.
    pub(crate) fn hex_width(self) -> usize {
        match self {
            ChecksumAlgorithm::Sha256 => 64,
        }
    }
}

/// Streams the file through the hasher rather than reading it into memory, so
/// large tracked artifacts (lockfiles, vendored bundles) don't cost RAM.
pub(crate) fn file_checksum(
    path: &Path,
    algorithm: ChecksumAlgorithm,
) -> Result<String, std::io::Error> {
    let mut file = fs::File::open(path)?;
    match algorithm {
        ChecksumAlgorithm::Sha256 => {
            let mut hasher = Sha256::new();
            std::io::copy(&mut file, &mut hasher)?;
            Ok(format!("{:x}", hasher.finalize()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdata::DIGEST_ABC;

    #[test]
    fn hashes_a_file_and_fails_on_a_missing_one() {
        let dir = std::env::temp_dir().join("ruleman_test_checksum");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        fs::write(&file, "abc").unwrap();

        assert_eq!(
            file_checksum(&file, ChecksumAlgorithm::Sha256).unwrap(),
            DIGEST_ABC
        );
        assert_eq!(DIGEST_ABC.len(), ChecksumAlgorithm::Sha256.hex_width());
        assert!(file_checksum(&dir.join("missing.txt"), ChecksumAlgorithm::Sha256).is_err());

        fs::remove_dir_all(&dir).unwrap();
    }
}
