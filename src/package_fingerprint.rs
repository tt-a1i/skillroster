use std::ffi::OsStr;
use std::io;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

pub(crate) const MAX_SKILL_FILE_BYTES: u64 = 2 * 1024 * 1024;
pub(crate) const MAX_SKILL_PACKAGE_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const MAX_SKILL_PACKAGE_DEPTH: usize = 8;

pub(crate) fn ignored_package_entry_name(name: &OsStr) -> bool {
    matches!(
        name.to_str(),
        Some(".git" | "target" | "node_modules" | ".DS_Store")
    )
}

pub(crate) fn normalized_relative_package_path(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "package path must be relative",
        ));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => normalized.push(name),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "package path must contain only normal relative components",
                ));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "package path must not be empty",
        ));
    }
    Ok(normalized)
}

pub(crate) struct PackageHashes {
    pub(crate) digest: String,
    pub(crate) content_identity_digest: String,
    pub(crate) skill_markdown: Option<String>,
}

pub(crate) struct PackageHashBuilder {
    digest: Sha256,
    content_identity: Sha256,
    total_bytes: u64,
    skill_markdown: Option<String>,
}

impl PackageHashBuilder {
    pub(crate) fn new() -> Self {
        Self {
            digest: Sha256::new(),
            content_identity: Sha256::new(),
            total_bytes: 0,
            skill_markdown: None,
        }
    }

    pub(crate) fn remaining_bytes(&self) -> u64 {
        MAX_SKILL_PACKAGE_BYTES.saturating_sub(self.total_bytes)
    }

    pub(crate) fn add_regular_file(&mut self, relative_path: &str, bytes: &[u8]) -> io::Result<()> {
        self.total_bytes = self.total_bytes.saturating_add(bytes.len() as u64);
        if self.total_bytes > MAX_SKILL_PACKAGE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("package exceeds {MAX_SKILL_PACKAGE_BYTES} byte fingerprint safety limit"),
            ));
        }
        update_regular_file_identity(&mut self.digest, relative_path, bytes);
        if relative_path != ".gitignore" {
            update_regular_file_identity(&mut self.content_identity, relative_path, bytes);
        }
        if relative_path == "SKILL.md" {
            if bytes.len() as u64 > MAX_SKILL_FILE_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Skill entrypoint exceeds {MAX_SKILL_FILE_BYTES} byte safety limit"),
                ));
            }
            self.skill_markdown = Some(String::from_utf8(bytes.to_vec()).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "Skill entrypoint is not UTF-8")
            })?);
        }
        Ok(())
    }

    pub(crate) fn add_symlink(&mut self, relative_path: &str, target: &str) {
        update_symlink_identity(&mut self.digest, relative_path, target);
        if relative_path != ".gitignore" {
            update_symlink_identity(&mut self.content_identity, relative_path, target);
        }
    }

    pub(crate) fn finish(self) -> PackageHashes {
        PackageHashes {
            digest: format!("{:x}", self.digest.finalize()),
            content_identity_digest: format!("{:x}", self.content_identity.finalize()),
            skill_markdown: self.skill_markdown,
        }
    }
}

fn update_regular_file_identity(digest: &mut Sha256, relative_path: &str, bytes: &[u8]) {
    digest.update(relative_path.as_bytes());
    digest.update([0]);
    digest.update(bytes);
    digest.update([0xff]);
}

fn update_symlink_identity(digest: &mut Sha256, relative_path: &str, target: &str) {
    digest.update(relative_path.as_bytes());
    digest.update([0]);
    digest.update(b"symlink\0");
    digest.update(target.as_bytes());
    digest.update([0xff]);
}
