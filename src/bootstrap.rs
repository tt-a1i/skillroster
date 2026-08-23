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

pub(crate) fn is_managed_target(relative_to_skill_root: &Path) -> bool {
    let Ok(relative_to_package) = relative_to_skill_root.strip_prefix("skillroster") else {
        return false;
    };
    PACKAGE_FILES
        .iter()
        .any(|file| relative_to_package == Path::new(file.relative_path))
}
