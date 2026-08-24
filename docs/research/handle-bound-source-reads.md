# Handle-bound durable source reads

> Issue: [#135](https://github.com/tt-a1i/skillroster/issues/135)
> Date: 2026-08-24
> Status: design investigation only; no product integration decision

## Executive decision

Issue #134's current contract is the right default for ordinary accidental or
persistent drift: resolve the confirmed path, compare the stored identity, and
check again around discovery/consumption. It is not a defence against a
same-user process that performs a precisely timed ABA replacement entirely
between those checks.

The smallest design that could address that stronger threat is a short-lived,
read-only **bound source session**. It opens the root once, traverses children
relative to an already-open directory/file object, reads and hashes from the
already-open handle, and never reopens a pathname for the result. The public
interface should expose capability and failure (`handle_bound`, `path_bound`,
or `unsupported`), not pretend that every platform has the same guarantee.

This document recommends designing that seam, but not adding it to the Rust
runtime yet. POSIX has a coherent descriptor-relative implementation. Windows
has strong handles and IDs, but the fully anchored child traversal choices are
split between a constrained Win32/ID implementation and the lower-level
`NtCreateFile` API. That portability and maintenance cost needs an explicit
product decision.

## Threat boundary

There are three different properties:

1. **Path drift:** a source root is moved, retargeted, deleted, or replaced
   between scans. #134 detects this with canonical path and filesystem identity.
2. **Bounded-checkpoint ABA:** an attacker replaces the object after the first
   check and restores an equivalent-looking object before the second check.
   Path checks and device/inode or volume/file-ID checks cannot make this
   guarantee.
3. **Handle-bound consumption:** each object used for discovery, package
   fingerprinting, and entrypoint reading is the object opened by the session,
   even if its pathname changes later.

The proposed design targets (3). It does not make a same-user process unable to
modify bytes through an already-open handle, nor does it create a snapshot of
all files in a mutable directory. A digest must therefore be computed from the
opened handle and validated against metadata captured from that same handle;
the result is “bytes read from this handle,” not an immutable filesystem
snapshot.

## Platform facts

### POSIX, Linux, and macOS

POSIX defines `openat()` as opening a relative path against a directory file
descriptor rather than the process current directory, specifically to avoid
pathname races. `fstat()` obtains status from the open descriptor, and
`fstatat()` supports descriptor-relative metadata queries. The standard also
defines `O_DIRECTORY` and the security rationale for `O_NOFOLLOW`.

- [POSIX `open`/`openat`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/open.html)
- [POSIX `fstat`](https://pubs.opengroup.org/onlinepubs/009696699/functions/fstat.html)
- [Linux `stat`/`fstatat`](https://man7.org/linux/man-pages/man2/lstat.2.html)
- [Linux `openat2`](https://man7.org/linux/man-pages/man2/openat2.2.html)
- [Apple/XNU `open(2)`](https://keith.github.io/xcode-man-pages/open.2.html)

Linux `openat2()` adds `RESOLVE_BENEATH`, `RESOLVE_IN_ROOT`,
`RESOLVE_NO_SYMLINKS`, and `RESOLVE_NO_MAGICLINKS`; these are useful when the
caller needs one kernel pathname-resolution operation with explicit containment
rules. It is Linux-specific (introduced in Linux 5.6), so it cannot be the
portable abstraction. macOS's documented `open(2)` page includes `O_NOFOLLOW`,
and newer Darwin headers expose stronger no-follow/beneath flags; availability
must be feature-detected rather than assumed from Linux names.

The minimum POSIX traversal is therefore:

```text
root = open(root_path, O_DIRECTORY | O_CLOEXEC | no-follow)
for each relative directory component:
    child = openat(parent_fd, component, O_DIRECTORY | O_CLOEXEC | no-follow)
    fstat(child) and retain child_fd
for each regular child:
    file = openat(parent_fd, name, O_RDONLY | O_CLOEXEC | no-follow)
    fstat(file), read(file), fstat(file) again
```

`O_NOFOLLOW` protects the final component only on the portable interface;
component-by-component opens (or Linux `openat2` / a supported Darwin stronger
flag) are required if intermediate links must be rejected. Symlinks are facts
to report, not package content to follow. Rust can pass Unix-specific flags via
[`OpenOptionsExt::custom_flags`](https://doc.rust-lang.org/std/os/unix/fs/trait.OpenOptionsExt.html),
but the descriptor lifetime and traversal policy still need a platform module.

### Windows

Win32 `CreateFile` returns a handle that remains valid for the object while a
reference is held. A directory can be opened with
`FILE_FLAG_BACKUP_SEMANTICS`; directory handles are accepted by a defined set of
APIs. `GetFileInformationByHandleEx(FileIdInfo)` supplies a volume serial and
128-bit file ID for comparing open handles. `FileAttributeTagInfo` exposes the
reparse tag, and `FILE_FLAG_OPEN_REPARSE_POINT` tells `CreateFile` not to follow
the final reparse point.

- [Win32 `CreateFileW`](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)
- [Directory handles](https://learn.microsoft.com/en-us/windows/win32/fileio/obtaining-a-handle-to-a-directory)
- [`FILE_ID_INFO`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_info)
- [`FILE_ATTRIBUTE_TAG_INFO`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_attribute_tag_info)
- [Reparse-point operations](https://learn.microsoft.com/en-us/windows/win32/fileio/reparse-point-operations)
- [`OpenFileById`](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-openfilebyid)

This is not the same API shape as POSIX `openat`: ordinary `CreateFileW` takes
a pathname, not a parent directory handle. `OpenFileById` can reopen an object
from a volume handle and file ID, but it is not a general portable
component-by-component traversal primitive and filesystem support varies.
Directory enumeration can return file IDs (`FileIdExtdDirectoryInfo`), but a
backend still has to open by ID, reject unsupported filesystems, inspect
reparse tags, and verify the returned handle before consuming it.

Windows Native System Services document `NtCreateFile` with a
`RootDirectory` handle and relative path names, plus `FILE_OPEN_BY_FILE_ID`.
That provides a closer semantic match, but it is a lower-level API with
filesystem-specific behavior and a materially larger compatibility/security
surface than the ordinary Win32 contract:

- [`NtCreateFile` relative `RootDirectory` and file-ID modes](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntcreatefile)
- [`NtQueryInformationFile` relative-name behavior](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntqueryinformationfile)

Consequently, a Windows implementation must fail closed when it cannot prove
that a child handle is anchored. A path reopen followed by an ID comparison is
still useful ordinary-drift detection, but it is not a handle-bound guarantee.

## Measured macOS prototype

Only a disposable `/tmp` directory was used; no repository, Skill, Agent file,
or persistent SkillRoster state was touched. The prototype opened `pkg` as a
directory descriptor, renamed that directory away, installed a different
directory at the old pathname, then read once by pathname and once relative to
the held descriptor.

Command shape (Python uses the host's `open(2)`/`openat(2)` through
`os.open(..., dir_fd=...)`):

```text
PROTO_DIR=$(mktemp -d /tmp/skillroster-135.XXXXXX)
PROTO_DIR="$PROTO_DIR" python3 - <<'PY'
# create pkg/SKILL.md and replacement/SKILL.md
# root_fd = os.open(root, O_RDONLY|O_DIRECTORY|O_NOFOLLOW)
# pkg_fd = os.open("pkg", O_RDONLY|O_DIRECTORY|O_NOFOLLOW, dir_fd=root_fd)
# rename pkg -> pkg-old; rename replacement -> pkg
# pathname = read(root / "pkg" / "SKILL.md")
# bound = read(os.open("SKILL.md", O_RDONLY|O_NOFOLLOW, dir_fd=pkg_fd))
PY
```

Observed on macOS 26.5.2 / Darwin 25.5 / arm64:

```json
{
  "root_fd_dir_survives_rename": true,
  "pathname_read_after_retarget": "replacement",
  "held_directory_fd_read": "original",
  "conclusion": "directory descriptor remains bound to original directory; pathname follows replacement"
}
```

This is a measured POSIX behavior demonstration, not an adversarial proof. No
Windows prototype was claimed: this host cannot validate Windows handle,
reparse, file-ID, or filesystem support behavior.

## Smallest future interface

Keep this behind one private filesystem seam; do not expose descriptors or
Windows handles in the CLI or SQLite. A possible semantic interface is:

```text
BoundSource::open(root_path, policy) -> BoundTree
BoundTree::entries(dir) -> sorted Entry { logical_relative_path, kind }
BoundTree::open_child(dir, name) -> BoundFile | BoundDir | Rejected
BoundFile::metadata() -> identity, type, size
BoundFile::read_all() -> bytes
BoundFile::metadata() -> identity, type, size   # post-read check
```

The implementation must retain the object, never follow symlinks/reparse
points for package content, reject special files, bound depth/entry count/size,
and return an explicit unsupported result if the platform cannot satisfy the
policy. `logical_relative_path` is for deterministic digest input and display;
it is not reopened to obtain bytes.

### Package fingerprints

Fingerprint input should be a sorted sequence of logical relative name, object
kind, size, and bytes read from each bound regular-file handle. A directory is
not content merely because it has a pathname. Capture metadata from the same
handle before and after reading; if type, size, or supported identity changes,
discard the fingerprint and report a typed source drift. This prevents a
same-size pathname replacement from being silently accepted, but it does not
promise an atomic multi-file snapshot under concurrent writes.

### Executable discovery

Discovery is read-only and must inspect bytes/metadata from the bound handle.
It should never execute a discovered file. A symlink/reparse candidate is
reported as excluded or unresolved, not dereferenced. If a future feature ever
executes a candidate, reopening the displayed path would lose the guarantee;
that would require a separate platform-specific execute-by-handle design and is
outside #135.

### Temporary `--source-root`

Keep the current one-shot path option unchanged. Internally, a scan may attempt
to create a bound session for each explicitly supplied root. Durable permissions
still store only local path/identity policy, never live handles. If the platform
or filesystem cannot provide the requested bound session, the scan must either
use the existing `path_bound` checkpoint contract and disclose that fact, or
fail closed when the caller explicitly requested the stronger mode. It must
not silently upgrade a path check into a handle guarantee.

## Options and recommendation

| Option | Value | Cost / rejection reason |
| --- | --- | --- |
| Keep path + pre/post identity checks | Correct for ordinary drift and current product scope | Does not close same-user ABA; retain as baseline |
| POSIX descriptor traversal + Windows handle/ID traversal | Smallest useful stronger seam; can fail closed per filesystem | Windows backend is non-symmetric; needs explicit support matrix |
| Use Windows `NtCreateFile` everywhere | Closest Windows analogue to `openat` | Native API, reparse and filesystem compatibility burden; not a first implementation |
| Copy the source tree to a private snapshot | Gives a stable read set | Changes read-only/no-copy semantics, adds storage, permissions, cleanup, and new mutation surface |
| Locks, oplocks, or sharing flags | Can reduce accidental concurrent interference | Not a general same-user adversarial guarantee; can be unavailable or defeated by same-user cooperation |
| Reopen path then compare identity | Useful drift check | The open itself remains pathname-racy; not handle-bound |

Recommendation: approve only a future capability-gated `BoundSource` experiment.
Implement POSIX first with deterministic tests, and make Windows return
`unsupported` until a reviewed backend can prove anchored child handles on the
target filesystem matrix. Do not add embeddings, a second scanner, a daemon,
or a new trust/configuration system for this security hardening.

## Phased interface and test plan

1. **Threat contract:** document whether the accepted guarantee is object-bound
   bytes or an atomic tree snapshot; define depth/size/special-file/reparse
   policy and the `path_bound` fallback output.
2. **POSIX prototype:** implement a private backend for macOS/Linux; use
   descriptor-relative opens, no-follow component policy, fstat-before/after,
   and explicit Linux `openat2` capability detection.
3. **Windows feasibility:** test directory enumeration, file IDs, reparse tags,
   `OpenFileById`, and (only if approved) `NtCreateFile` on NTFS, ReFS, and a
   filesystem where IDs are unsupported. No silent fallback in handle mode.
4. **Adapters:** route package digest and executable discovery through bound
   handles; preserve temporary `--source-root` and all current receipts/JSON.
5. **Acceptance:** publish platform-specific evidence before changing the
   ordinary default.

Deterministic future tests should include:

- rename/retarget after root and child directory opens;
- final and intermediate symlink insertion, deletion, and escape;
- same-size, same-mtime content replacement;
- deletion and replacement between enumeration and child open;
- hard links, directory reorder, depth/entry/size bounds, and special files;
- Linux `openat2` unavailable/unsupported and macOS stronger flags unavailable;
- Windows reparse tags, file-ID mismatch/reuse, share violations, NTFS/ReFS,
  unsupported filesystem behavior, and relative native-open failure;
- temporary `--source-root` with one root bound and one root unsupported;
- digest and executable-discovery outputs proving no pathname reopen;
- a negative test that a path-bound result is never labelled handle-bound.

## Sources

All normative platform claims above link to the owning standards or first-party
documentation. Rust's platform extension points are documented in the standard
library: [Unix `OpenOptionsExt`](https://doc.rust-lang.org/std/os/unix/fs/trait.OpenOptionsExt.html)
and [Windows `OpenOptionsExt`](https://doc.rust-lang.org/std/os/windows/fs/trait.OpenOptionsExt.html).
