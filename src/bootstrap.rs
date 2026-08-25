use std::path::Path;

#[derive(Clone, Copy)]
pub(crate) struct PackageFile {
    pub(crate) relative_path: &'static str,
    pub(crate) content: &'static str,
}

pub(crate) const PACKAGE_FILES: [PackageFile; 4] = [
    PackageFile {
        relative_path: "SKILL.md",
        content: include_str!("../skill/skillroster/SKILL.md"),
    },
    PackageFile {
        relative_path: "references/routing.md",
        content: include_str!("../skill/skillroster/references/routing.md"),
    },
    PackageFile {
        relative_path: "references/governance.md",
        content: include_str!("../skill/skillroster/references/governance.md"),
    },
    PackageFile {
        relative_path: "references/mutation.md",
        content: include_str!("../skill/skillroster/references/mutation.md"),
    },
];

pub(crate) fn content_version() -> Option<&'static str> {
    let content = PACKAGE_FILES
        .iter()
        .find(|file| file.relative_path == "SKILL.md")?
        .content;
    parse_content_version(content)
}

fn parse_content_version(content: &str) -> Option<&str> {
    let mut lines = content.lines();
    if lines.next()? != "---" {
        return None;
    }

    let mut in_metadata = false;
    let mut metadata_child_indentation = None;
    let mut version = None;
    for line in lines {
        if line == "---" {
            return version;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indentation = line.len().saturating_sub(line.trim_start().len());
        if indentation == 0 {
            in_metadata = trimmed == "metadata:";
            metadata_child_indentation = None;
            continue;
        }
        if !in_metadata {
            continue;
        }
        let child_indentation = *metadata_child_indentation.get_or_insert(indentation);
        if indentation != child_indentation {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        if key.trim() != "bootstrap-version" || version.is_some() {
            continue;
        }
        version = parse_version_scalar(value.trim());
    }
    None
}

fn parse_version_scalar(value: &str) -> Option<&str> {
    let value = match value.as_bytes() {
        [b'"', .., b'"'] | [b'\'', .., b'\''] => &value[1..value.len() - 1],
        _ => value,
    };
    let mut parts = value.split('.');
    let valid = (0..3).all(|_| {
        parts
            .next()
            .is_some_and(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    }) && parts.next().is_none();
    valid.then_some(value)
}

pub(crate) fn is_managed_target(relative_to_skill_root: &Path) -> bool {
    let Ok(relative_to_package) = relative_to_skill_root.strip_prefix("skillroster") else {
        return false;
    };
    PACKAGE_FILES
        .iter()
        .any(|file| relative_to_package == Path::new(file.relative_path))
}

#[cfg(test)]
mod tests {
    use super::{content_version, parse_content_version};

    #[test]
    fn content_version_comes_from_the_bundled_skill_frontmatter() {
        assert_eq!(content_version(), Some("1.8.28"));
    }

    #[test]
    fn content_version_requires_closed_leading_frontmatter_and_metadata_parent() {
        assert_eq!(
            parse_content_version(
                "---\r\nname: fixture\r\nmetadata:\r\n  bootstrap-version: '1.8.23'\r\n---\r\nBody"
            ),
            Some("1.8.23")
        );
        assert_eq!(
            parse_content_version("# preface\n---\nmetadata:\n  bootstrap-version: 1.8.23\n---"),
            None
        );
        assert_eq!(
            parse_content_version("---\nmetadata:\n  bootstrap-version: 1.8.23"),
            None
        );
        assert_eq!(
            parse_content_version(
                "---\nother:\n  bootstrap-version: 9.9.9\nmetadata:\n  nested:\n    bootstrap-version: 8.8.8\n---\nBody\n---\nbootstrap-version: 7.7.7"
            ),
            None
        );
    }

    #[test]
    fn content_version_rejects_non_version_scalars() {
        for value in ["", "1.8", "1.8.23-beta", "1.8.23 # comment", "\"1.8.23"] {
            let content = format!("---\nmetadata:\n  bootstrap-version: {value}\n---\n");
            assert_eq!(parse_content_version(&content), None, "value: {value}");
        }
    }
}
