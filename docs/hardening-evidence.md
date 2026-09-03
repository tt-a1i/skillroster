# Release hardening evidence and boundaries

This round addresses [#370](https://github.com/tt-a1i/skillroster/issues/370),
[#371](https://github.com/tt-a1i/skillroster/issues/371), and
[#372](https://github.com/tt-a1i/skillroster/issues/372), from main `2beab5d`.
Historical published assets remain separately tracked in
[#55](https://github.com/tt-a1i/skillroster/issues/55). Independent user-pilot
work is outside this round.

## Performance

[The acceptance record](acceptance.md#synthetic-overlap-scale-baseline-release-hardening-round)
contains the reproducible synthetic workload, machine, before/after timing and
process peak RSS. The deterministic gate proves at most 25 retained semantic
candidates, exact pair counts, and identical output against the original
full-sort algorithm. Pair enumeration remains quadratic. There is no universal
five-second report guarantee.

## Recovery tests

The ordinary Rust suite includes `storage_errno_failures_keep_a_durable_recovery_boundary`.
It injects ENOSPC/EIO on Unix, and ERROR_DISK_FULL/ERROR_IO_DEVICE on Windows,
at the initial journal-directory barrier and at the created target's directory
barrier, in both one-shot and persistent forms. It checks whether target bytes
exist, that journal evidence remains readable, and that a later Apply refuses
to proceed past unresolved recovery. This is an injected directory-sync errno
test, not a physically full filesystem or an emulated failing block device.

`killed_apply_blocks_new_writes_after_process_restart` runs the real Apply
implementation in a separate test process. The parent waits for a concrete
checkpoint after initial journal publication or target-file publication,
terminates that child (SIGKILL on Unix), waits for its death, and starts a fresh
process against the same disposable state. The restarted process must observe
the Applying journal and reject a new write. Journal bytes and the expected
target bytes are compared across restart. Hooks and subprocess environment
keys exist only in the test build, not the shipped CLI.

These tests run through the normal full CI/release Rust suites. A green result
establishes only the exercised software boundaries. It does **not** simulate
loss of filesystem/device caches, controller write reordering, power failure,
or a disk that acknowledges durability without providing it. Do not call this
physical power-loss certification. Existing directory synchronization,
compensation, SQLite finalization, and lifecycle-reconciliation tests remain
separate gates; process death does not replace them.

## Copy and replacement metadata

Copy-based mutations inspect metadata using retained source/destination file
handles. A copy may now refuse a filesystem layout that previously lost
metadata silently. There is no override that discards unsupported metadata.

| Platform | Supported copy metadata | Refused or deliberately outside the guarantee |
| --- | --- | --- |
| Linux | Current-user owner, group, permission bits; directory modes finalized after children | Observable xattrs, including POSIX ACLs; failed metadata inspection |
| macOS | Current-user owner, group, permission bits; identical OS-created provenance bytes | Other xattrs, extended ACLs, nonzero file flags, changed provenance |
| Windows | Ordinary/readonly attributes and exactly matching owner/group/DACL, including inheritance control | Different destination ACL defaults, named streams, EAs, other file attributes |

Windows recovery backups deliberately retain owner-only access. Original
owner/group/DACL evidence is stored as an additive private Receipt field, not
applied to the backup. Before bytes are copied into a restoration staging file,
its security descriptor must match that evidence. SkillRoster never changes a
destination DACL to force a match. Legacy Windows **replacement** Receipts with
no original security evidence cannot establish that comparison and are refused;
their files and recovery evidence remain for explicit manual recovery. This
does not invalidate non-replacement Undo operations or Unix Receipts.

Replacement Undo also compares the current observable metadata with recovery
evidence. A changed mode, owner/group, visible xattr/ACL or Windows DACL is a
recovery boundary, not permission to overwrite the user's change.

This does not claim to preserve timestamps, sparse allocation, hard-link
topology, privileged/invisible metadata, filesystem-specific policy, or Windows
SACL audit settings. An application requiring those properties needs a native
metadata-aware backup/migration workflow. Read-only Scan/Report/Find remain
available for such files; observation is not mutation authorization.

Reference contracts: [Linux handle-based xattr enumeration](https://man7.org/linux/man-pages/man2/listxattr.2.html)
may omit attributes inaccessible to the caller; this is why absence cannot
prove privileged metadata is absent. Darwin distinguishes missing ACL
properties from empty ACL objects in its
[ACL handle implementation](https://github.com/apple-oss-distributions/Libc/blob/main/posix1e/acl_file.c)
and [filesec property implementation](https://github.com/apple-oss-distributions/Libc/blob/main/gen/filesec.c).
Windows [file security](https://learn.microsoft.com/en-us/windows/win32/fileio/file-security-and-access-rights)
is attached to the file itself; a private parent directory is not a substitute
for protecting a recovery file's DACL.

## Focused simplification

The former private `AnchoredFs::copy_file` wrapper had no remaining production
callers after backup/restore metadata became explicit. Its two tests now use
the production `copy_tree` entrypoint, which dispatches regular files to the
same retained-handle implementation. No containment, identity, durability or
rollback guard was removed. The source diff/commit can be reverted independently
of any released artifacts; reverting source does not repair metadata already
lost by an older binary or restore deleted release assets.

## Cross-platform regression checks

Independent review and the first platform CI run caught two handle-lifecycle
regressions before merge. Linux directory capabilities may use `O_PATH`, which
cannot inspect xattrs; Windows read-only directory handles cannot apply metadata
or flush it. Copy now opens explicitly readable source directories and
metadata-write/flush-capable destination directories, retaining no-follow and
directory-type checks. `recursive_directory_copy_and_undo_round_trip` runs on
every platform, in addition to the Unix readonly-directory test. On Windows,
long-lived copy handles also retain no-delete-sharing protection for source
and destination directories; they are deliberately separate from short-lived
parent-sync handles. The test attempts to rename both directory trees and
their nested directories during child-file copying and requires refusal.

On Windows, replacement also closes its retained original-file handles after
validated removal and before publishing the staged file. Otherwise the delete
disposition stays pending and the no-replace rename correctly refuses an
apparently occupied destination. The existing readonly Replace/Undo, complete
Bootstrap upgrade/Undo, and full operation-ledger tests exercise that lifecycle.
See Microsoft's [delete-disposition contract](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntddk/ns-ntddk-_file_disposition_information_ex).
