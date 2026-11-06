# Changelog

## 0.1.0 - 2026-11-05

Initial release.

- Feature: `slinky` finds the symlinks under a path and acts on each one. Its
  commands list, tidy, canonicalize, and rewrite link targets
  (`tidy-target`, `canonicalize`, `to-relative`, `to-absolute`,
  `edit-target`); replace links with hardlinks, trees of links, or their
  targets (`to-hardlink`, `to-tree`, `replace-with-target`); remove links
  (`remove`, which needs `--force`); and run a shell command on each link
  (`exec`). Filters select links by state, form, origin, target, and depth.
- Feature: `slinky-ln`, an alternative to `ln(1)` with one invocation form.
  It can write absolute, relative, or literal link text, dereference the
  target, make hardlinks, and mirror a directory as a tree of links.
- Packaging: prebuilt binaries for x86_64 and aarch64 Linux and macOS, a
  Homebrew formula (`fictionic/tap/slinky`), and the AUR packages `slinky` and
  `slinky-bin`. Each one includes man pages and bash, zsh, and fish
  completions.
