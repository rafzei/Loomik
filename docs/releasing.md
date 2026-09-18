# Release packaging and publication

Publish only artifacts that pass automated build/package verification. Release
notes must distinguish CI results from physical capture tests; current evidence
and remaining hardware checks are recorded in [VERIFICATION.md](../VERIFICATION.md).
Released assets are immutable. Security fixes require a new version, release
notes and verified packages; never replace the existing 1.0.0 downloads.

## Downloads

| Target | Assets |
| --- | --- |
| macOS 13+, Apple Silicon | `Loomik-1.1.0-macOS-arm64.zip` and `.dmg` |
| macOS 13+, Intel | `Loomik-1.1.0-macOS-x86_64.zip` and `.dmg` |
| Windows 11 x64 | `Loomik-1.1.0-Windows-x64.zip` |
| Ubuntu 24.04 LTS amd64 | `Loomik-1.1.0-Ubuntu-24.04-amd64.deb` |

macOS/Windows packages include FFmpeg/FFprobe, licenses, corresponding source
archives and build recipes. Ubuntu installs runtime dependencies through APT.
Rust dependency notices are included. macOS packages are ad-hoc signed, not
notarized; Windows packages are unsigned. Installation instructions describe the
resulting OS prompts.

## Publication gates

- Native GitHub Actions matrix passes on Apple Silicon, Intel, Windows MSVC
  and Ubuntu, using the exact commit to tag.
- Rust formatting, Clippy and tests pass. macOS/Windows tests use bundled media
  tools. ZIP extraction preserves executable modes/signatures and succeeds from
  paths containing spaces. Packaged executables export and decode H.264/AAC in
  MP4/MOV/MKV with only system directories on PATH.
- Ubuntu builds/installs the `.deb`, runs GUI smoke checks under Xvfb and verifies
  isolated XComposite window pixels beneath occluding controls. These tests do
  not substitute for Wayland portal or physical device checks.
- Archive SHA-256 checksums match downloaded Actions artifacts. Attach those
  exact tested archives to GitHub Release `v1.1.0`, with a combined checksum file.
- Release notes describe actual verification and known limits: Linux window-only
  capture/no cursor, compositor-dependent window positioning, unsigned/notarized
  status, and outstanding Windows/Ubuntu hardware measurements.

## Publish verified packages

Application ZIP/DMG/DEB downloads live in **GitHub Releases**. Actions artifacts
are temporary build outputs.

1. Wait for all four jobs in `Native builds and verified packages` to pass on a
   push to `main`. Record the run ID. Prepare `docs/releases/X.Y.Z.md` and the
   matching Cargo version before that build.
2. As `rafzei`, open Actions → **Publish verified release packages** → Run workflow,
   choose `main`, and supply the successful build's `run_id`. The CLI equivalent is:
   `gh workflow run publish-release.yml --repo rafzei/Loomik --ref main -f run_id=RUN_ID`.
3. The read-only preparation job checks the source repository, event, workflow,
   commit ancestry, all four jobs, package reports and SHA-256 checksums. It downloads
   the exact tested packages, never executes them, and stages 14 release assets.
4. Only the publication job receives `contents: write`. It creates a draft, uploads
   the verified assets, compares GitHub's SHA-256 digests, and then publishes the
   release and tag for the tested commit. Existing tags or files with different
   contents cause failure. Rerunning an identical published release verifies it
   without replacing downloads or release notes.

## Public repository restrictions

- Builds run on pushes to `main` and PRs targeting `main`. Repository settings
  require approval for **all external contributors' fork PR workflows**, including
  repeat contributors. Review the PR's workflow and scripts before approving.
- Manual builds and publication are restricted to `rafzei` on `main`, including
  reruns (`github.triggering_actor`). Workflows are disabled in forks by repository
  identity checks. There are no `pull_request_target` or automatic publication triggers.
- The `release` environment allows only the `main` branch. No extra deployment
  approval is needed after the owner explicitly starts publication.
- Default workflow tokens are read-only and cannot approve PR reviews. Checkout
  does not persist credentials. PR jobs do not save shared caches. Publication
  uses the workflow's token, with no personal access token or package-registry token.
- External actions are pinned to full commit SHAs. Repository policy requires SHA
  pinning and allows only the actions listed in these workflows. Changing an action
  requires updating both its pinned revision and, for new actions, the allowlist.
- Concurrency cancels superseded builds and serializes release publication.

Repository settings and environment restrictions are configured in GitHub as well
as workflow code; preserve them when moving or recreating the repository.
