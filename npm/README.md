# SkillRoster npm distribution

The public npm release is v1.9.1. The distribution contains one `skillroster` launcher and four exact-version
optional binary packages.

```sh
npx -y skillroster check
claude mcp add --transport stdio skillroster -- npx -y skillroster mcp
codex mcp add skillroster -- npx -y skillroster mcp
```

Node 18+ runs the launcher. The CLI and MCP server are native Rust; no Rust
toolchain is needed by npm users. Supported targets match the four Release
archives in `platforms.json`. Linux requires glibc; WSL requires WSL2.
Other architectures have no fallback download. Do not omit optional dependencies.
The launcher verifies its binary's packaged SHA-256 and preserves stdio and exit
status. There is no install hook and no network request from the launcher.
The npm client may use the network to retrieve packages.

For release maintainers (Python 3, Node and npm needed only for packaging):

1. Build and approve all four archives with the existing Release workflow at
   the intended source SHA. Download each archive and adjacent `.sha256` into
   one local directory. Old v1.8.48 public archives do not contain `check`,
   `init`, or MCP; use artifacts built from this upgraded source for its release.
2. Generate without network access, choosing a new output directory:

   ```sh
   python3 scripts/release/build-npm.py --version VERSION --artifacts /path/to/archives --output /path/to/npm-stage
   bash scripts/verify-npm-package.sh
   ```

   The version must match the source version (or its `-rc` prerelease).
   The generator verifies every archive hash before reading its exact regular
   binary member. It writes binary hashes, full licenses, exact-version
   dependencies, and archive/target provenance. A checksum mismatch, missing
   target, symlink member, or existing destination fails without publishing.
3. Run `npm pack --ignore-scripts` in every staged package and inspect
   `npm pack --dry-run --json --ignore-scripts`. Test a local installation and
   piped MCP session on each target before publishing. Verify npm name/scope
   ownership and reserve the new release version first. Publish all four
   platform packages before the main package; prereleases use a non-latest tag.
   Publishing and registry credentials are separate maintainer actions.

SHA-256 ties packages to the supplied approved artifacts; it is not an
independent signature or proof of content safety. Repository tests use tiny
synthetic archives to test packaging, not to claim cross-platform execution.
