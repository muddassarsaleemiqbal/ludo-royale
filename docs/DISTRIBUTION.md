# Distribution guide

Ludo Royale ships through GitHub Releases. Every push and pull request is
checked by CI. A successful CI run for a push to `main` starts the release
workflow, increments the patch version, builds the installers, commits the
new version, creates an annotated tag, and publishes the release.

## Release outputs

Each release contains:

- a universal Apple Silicon + Intel macOS DMG;
- a 64-bit Windows NSIS setup executable;
- a 64-bit Windows MSI;
- a CycloneDX 1.5 JSON software bill of materials;
- SHA-256 checksums for every published file;
- GitHub-generated release notes and source archives.

The root `Cargo.toml` is the single version source. The Rust `xtask` utility
calculates and applies `major.minor.patch` versions without duplicating them in
packaging configuration.

## Repository settings

Before the first release:

1. Set `main` as the default branch.
2. In **Settings → Actions → General → Workflow permissions**, allow read and
   write access so the release workflow can commit, tag, and create a release.
3. Require the `CI / Rust core — compiler, Clippy, tests, and docs`,
   `CI / macOS desktop — compiler, Clippy, tests, and docs`,
   `CI / Windows desktop/network — compiler and tests`, and
   `CI / Dependency security audit` checks in the `main` branch ruleset.
4. Enable private vulnerability reporting under **Settings → Security**.

If the branch ruleset does not allow `github-actions[bot]` to create the
version commit, create a fine-grained personal access token or GitHub App token
with repository **Contents: read and write**, permit that identity to bypass
the release-only rule, and save it as `RELEASE_TOKEN`. Without branch
protection, the built-in `GITHUB_TOKEN` is sufficient and is preferred.

The release job checks that `main` still points to the exact commit that passed
CI before it writes a version. If another push wins that race, the older
release stops and the newer push's CI run becomes the release candidate.

The RustSec check fails for new vulnerabilities, unsound crates, and yanked
dependencies. Two documented `quick-xml` advisories are temporarily ignored in
`.cargo/audit.toml`: that version is pinned by the current `xcb` release, runs
only while building GPUI's optional Linux backend, parses trusted xcb-proto
build files, and is not present in either distributed application. Existing
unmaintained GPUI transitive crates remain visible as warnings.

## CI performance model

Public repositories receive GitHub's stronger standard machines at no charge.
CI uses the 4-CPU/16-GB Ubuntu runner for platform-independent crates, the
4-CPU/14-GB `macos-15-intel` runner for GPUI's macOS target, and the
4-CPU/16-GB Windows runner for the desktop and LAN targets.

The core, macOS, Windows, and security jobs run in parallel. Each platform keeps
related compiler, Clippy, test, and documentation steps in one job so they
share that runner's Cargo target directory. Binary build artifacts cannot be
shared safely between operating systems. The local `setup-rust` composite
action centralizes toolchain setup and uses dependency-only Rust caches across
runs; it deliberately excludes workspace outputs and incremental artifacts to
keep cache transfer time and the repository's cache footprint bounded.

The Ubuntu core job installs `libasound2-dev` because `ludo-infrastructure`
uses Rodio, whose CPAL backend requires ALSA development files when compiled on
Linux. This system package is deliberately scoped to that one job.

There is no separate `cargo check` immediately before Clippy because Clippy
already invokes the Rust compiler. Likewise, `cargo test` is the Windows
compiler gate because Cargo must compile every selected target before running
it. This preserves compiler coverage without compiling the same targets twice.

## Apple signing and notarization

Unsigned DMGs are produced when Apple secrets are absent. Public downloads
should be signed with a **Developer ID Application** certificate and notarized
to avoid Gatekeeper warnings.

Add these GitHub Actions repository secrets:

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | Base64-encoded `.p12` Developer ID Application certificate |
| `APPLE_CERTIFICATE_PASSWORD` | Password used when exporting the `.p12` |
| `APPLE_ID` | Apple developer account email |
| `APPLE_PASSWORD` | App-specific password for that Apple ID |
| `APPLE_TEAM_ID` | Ten-character Apple Developer team ID |

Encode a certificate on macOS:

```sh
base64 -i DeveloperIDApplication.p12 | pbcopy
```

When all secrets are present, the workflow signs the universal app and DMG,
submits the app and final DMG to Apple's notarization service, staples the
ticket, and validates the result.

## Windows signing

Unsigned NSIS and MSI installers are produced when Windows secrets are absent.
For public distribution, obtain an Authenticode code-signing certificate and
add:

| Secret | Value |
| --- | --- |
| `WINDOWS_CERTIFICATE_BASE64` | Base64-encoded `.pfx` code-signing certificate |
| `WINDOWS_CERTIFICATE_PASSWORD` | Password for the `.pfx` |

Encode a certificate in PowerShell:

```powershell
[Convert]::ToBase64String(
    [IO.File]::ReadAllBytes("ludo-royale-code-signing.pfx")
) | Set-Clipboard
```

The workflow imports the certificate into the runner's temporary user store.
`cargo-packager` signs the application, NSIS installer, and MSI with SHA-256
and an RFC 3161 timestamp, after which the workflow verifies both installer
signatures.

## Local packaging

Install the pinned packager:

```sh
cargo install cargo-packager --version 0.11.8 --locked
```

Build a universal DMG on macOS:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build -p ludo-desktop --release --target aarch64-apple-darwin
cargo build -p ludo-desktop --release --target x86_64-apple-darwin
mkdir -p target/universal-apple-darwin/release
lipo -create \
  target/aarch64-apple-darwin/release/ludo-desktop \
  target/x86_64-apple-darwin/release/ludo-desktop \
  -output target/universal-apple-darwin/release/ludo-desktop
cargo packager --release --target universal-apple-darwin \
  --formats dmg --packages ludo-desktop
```

Build both Windows installers from a Developer PowerShell prompt:

```powershell
cargo build -p ludo-desktop --release
cargo packager --release --formats nsis,wix --packages ludo-desktop
```

The checked-in `.icns`, multi-resolution `.ico`, and 1024-pixel PNG source are
under `crates/ludo-desktop/assets/icons`.
