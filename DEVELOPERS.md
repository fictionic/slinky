# Developers

## Setup

1. Install a Rust toolchain with [rustup][]. CI uses the current stable
   release, so keep yours up to date with `rustup update`; a newer clippy can
   fail CI on code that passes locally.
2. Make sure the tests pass: `cargo test`.

For releases, you also need:

- [gh][], logged in to GitHub
- `makepkg`, for the AUR packages (Arch Linux only)
- [dist][], only to change the release setup (see below). Install it with
  `cargo binstall cargo-dist` or `cargo install cargo-dist --locked`.

[rustup]: https://rustup.rs/
[gh]: https://cli.github.com/
[dist]: https://github.com/axodotdev/cargo-dist

## Testing

`cargo test` runs the unit tests in `src/` and the integration tests in
`tests/`. The integration tests run the built binaries, and make their
assertions only on output, exit codes, and the filesystem.

CI runs these on Linux and macOS for each push to `main` and each pull
request. Run them before you push:

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

Some tests skip themselves when the system cannot run them: the permission
tests skip as root, and the test for a non-UTF-8 file name skips on a
filesystem that refuses one (such as APFS on macOS). A skipped test passes,
and `cargo test` hides its message.

`build.rs` generates the man pages and the shell completions into
`generate/`.

## Making a release

1. Raise the version in `Cargo.toml`, and run `cargo build` to update
   `Cargo.lock`. Add a heading for the version at the top of `CHANGELOG.md`,
   in the form `## X.Y.Z - YYYY-MM-DD`, with its entries below. dist uses
   that entry as the release notes.

2. Commit, push `main`, and wait for CI to pass.

3. Check that the release is ready:

   ```sh
   maint/release check
   ```

   This checks that the tree is clean and on `main`, that `Cargo.lock` is up
   to date, that the tag does not exist yet, that `CHANGELOG.md` has the
   version, that `HEAD` is pushed, and that CI passed on it.

4. Tag the release and push the tag:

   ```sh
   maint/release build tag
   maint/release push tag
   ```

   `build tag` runs the checks again first. The tag starts the Release
   workflow (dist), which builds the binaries for each target, creates the
   GitHub release, and publishes the Homebrew formula to
   `fictionic/homebrew-tap`. A workflow in the tap then patches the formula
   to install the man pages and the shell completions.

   Push `main` before the tag, as the checks require. The workflow cannot
   create a release for a commit that is on no branch: GitHub refuses it with
   `HTTP 403: Resource not accessible by integration`.

5. Update the AUR packages, after the GitHub release is published:

   ```sh
   maint/release build aur
   maint/release push aur
   ```

   `build aur` sets the version and the checksums in the `PKGBUILD` of
   `slinky` and `slinky-bin`, regenerates `.SRCINFO`, builds each package with
   `makepkg` in a temporary directory, and commits. Read the commits over
   before `push aur`. The AUR repositories are expected next to this one, at
   `../aur-slinky` and `../aur-slinky-bin`; set `AUR_SLINKY_DIR` and
   `AUR_SLINKY_BIN_DIR` to use other paths.

6. Check an install, for example `brew upgrade slinky` on a Mac.

## Changing the release setup

dist generates `.github/workflows/release.yml`. Do not edit it by hand:
change `dist-workspace.toml`, then run `dist generate`.

`cargo-dist-version` in `dist-workspace.toml` sets the dist version that CI
uses, and dist refuses to run locally at any other version. To upgrade,
install the new dist, set `cargo-dist-version` to match, and run
`dist generate`.

dist chooses a GitHub runner for each target. When GitHub retires a runner
that dist still chooses, the build for that target fails at once with "The
configuration ... is not supported". Upgrade dist, or override the runner in
`dist-workspace.toml`:

```toml
[dist.github-custom-runners]
aarch64-apple-darwin = "macos-latest"
```
