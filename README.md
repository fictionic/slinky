# slinky

> A utility for wrangling symbolic links on Unix systems.

The programs provided are `slinky(1)`, which finds and transforms symlinks in
bulk, and `slinky-ln(1)`, an alternative to `ln(1)`.

## Why?

Symlinks are great, but if you're like me, you occasionally find yourself
needing to "refactor" a large number of them at once. For example, you might
want to:

- Convert absolute symlinks to relative symlinks
- Canonicalize the target filesystem paths within symlinks

These tasks would be a pain to do with a custom shell script. The classic
[symlinks](https://github.com/brandt/symlinks) utility does handle them;
however, I found its features limited for my purposes. For example, imagine you
want to:

- Convert relative symlinks to absolute symlinks
- Convert a symlink of a directory into a directory containing symlinks of the
  directory's contents
- Replace all symlinks within a given directory with hardlinks to their targets
- Given a bunch of symlinks that point into a given directory, rename that
  directory without breaking the symlinks

`slinky` can do all of these things.

> BONUS! Slinky also comes with a binary called `slinky-ln`, which offers most
> of the same behavior of classic `ln` but with a more clear CLI. See below.

(Obligatory "written in Rust", "blazing-fast", etc.)

## Installation

MacOS (Homebrew):

```sh
brew install fictionic/tap/slinky
```

Arch Linux -- a `PKGBUILD` is included:

```sh
git clone https://github.com/fictionic/slinky
cd slinky
makepkg -si
```

From a release tarball -- prebuilt for x86_64 and aarch64 Linux and macOS:

```sh
TARGET=x86_64-unknown-linux-gnu   # or aarch64-unknown-linux-gnu, x86_64-apple-darwin, aarch64-apple-darwin
curl -LO https://github.com/fictionic/slinky/releases/download/v$VERSION/slinky-$TARGET.tar.xz
tar -xf slinky-$TARGET.tar.xz
sudo install -m 755 slinky-$TARGET/slinky slinky-$TARGET/slinky-ln /usr/local/bin/
```

Man pages and shell completions are in `slinky-$TARGET/generate/`.

From source -- needs a Rust toolchain:

```sh
git clone https://github.com/fictionic/slinky
cd slinky
cargo build --release
```

The binaries land in `target/release/`, and the man pages and shell
completions in `generate/`.

Homebrew and the `PKGBUILD` install both binaries along with the man pages and
the shell completions for bash, zsh, and fish.

## Usage

```
Usage: slinky [OPTIONS] [PATH] <COMMAND>

Commands:
  list                 List symlinks [aliases: ls]
  tidy-target          Remove redundant elements from symlink target paths
  canonicalize         Convert symlink paths into their canonical form (absolute path, with all intermediate symlinks resolved). Skips dangling symlinks
  to-relative          Convert absolute symlinks to relative symlinks
  to-absolute          Convert relative symlinks to absolute symlinks
  edit-target          Edit the target string of symlinks by replacing regex matches
  to-hardlink          Convert symlinks to hardlinks. Skips dangling symlinks, symlinks to directories, and cross-device symlinks
  to-tree              Convert a directory symlink into a directory tree of symlinks to files. Skips dangling symlinks and symlinks to non-directories
  replace-with-target  Move the target to the symlink's location. Skips dangling symlinks and cross-device symlinks
  remove               Remove symlinks [aliases: rm]
  exec                 Run a shell command against symlinks
  help                 Print this message or the help of the given subcommand(s)

Arguments:
  [PATH]  The path in which to search for symlinks [default: .]

Options:
  -x, --only-dangling           Only act on dangling symlinks
  -a, --only-attached           Only act on 'attached' (non-dangling) symlinks
  -b, --only-absolute           Only act on absolute symlinks
  -r, --only-relative           Only act on relative symlinks
  -o, --filter-origin <FILTER>  Only act on symlinks whose origin path matches the given regex
  -t, --filter-target <FILTER>  Only act on symlinks whose target string matches the given regex
  -d, --max-depth <NUM>         Descend at most NUM directories
  -v, --verbose                 Describe any changes to be made
  -n, --dry-run                 Don't make any changes
  -h, --help                    Print help
  -V, --version                 Print version
```

## Examples

List all relative symlinks under the directory `./dir`, and print their status:

```console
$ slinky -r ./dir list --status
attached: ./dir/foo/bar -> ../../baz
dangling: ./dir/qux -> ../../doesnt-exist
```

Replace every instance of `/old/path` with `/new/path` in absolute symlinks,
and be verbose:

```console
$ slinky -b -v edit-target '/old/path' '/new/path' --replace-all
edit-target: ./foo/bar -> (/mnt/foo/old/path/bar => /mnt/foo/new/path/bar)
edit-target: ./foo/baz -> (/mnt/foo/old/path/baz => /mnt/foo/new/path/baz)
```

Rename a directory, and update the symlinks that point into it (the pattern
also matches a link to the directory itself, but not one to `/data/photos-old`):

```console
$ mv /data/photos /data/pictures
$ slinky -v edit-target '^/data/photos(/|$)' '/data/pictures$1'
edit-target: ./cat.jpg -> (/data/photos/cat.jpg => /data/pictures/cat.jpg)
edit-target: ./album -> (/data/photos => /data/pictures)
```

Replace a directory symlink with a tree of symlinks:

```console
$ slinky ./foo to-tree
```

(Each link in the tree keeps the form of the original link's target: a relative
link gives a tree of relative links, and an absolute link gives a tree of
absolute links.)

Run an arbitrary command against each symlink using `/bin/sh` (`$1` is bound to
the link origin, and `$2` is bound to the link target):

```console
$ slinky exec 'echo "$1 points to $2"'
./foo/bar points to ../../baz
```

## Bonus: Create Symlinks

I can never remember how to use the `ln` command. I find its interface, and the
explanation in its man page, to be quite confusing. (Why does its invocation
have four forms???)

To address this, I created an alternative to `ln` called `slinky-ln`, which
has most of the same features, plus some bonus ones, and an improved interface
and manual.

### `slinky-ln` Usage

```
Usage: slinky-ln [OPTIONS] <TARGET> [ORIGIN]

Arguments:
  <TARGET>  The path to the file that the link will point to. A relative path resolves from the current directory, as the origin does
  [ORIGIN]  The path where the link will live. If a directory is provided, the link will be created inside that directory with the same basename as the target [default: .]

Options:
  -f, --force        Force creation of the link by overwriting existing files. Will not overwrite directories
  -b, --absolute     Write an absolute path to the target. Symlinks in the path are retained
  -r, --relative     Write a relative path to the target, from the link's directory. Symlinks in the target path are retained
  -L, --dereference  Dereference the target file if it is a symbolic link. The link text keeps the form of the given path
      --literal      Write the symlink target exactly as given by TARGET, without attempting to resolve it as a file path
  -H, --hard         Create a hardlink instead of a symlink
  -T, --tree         Create a tree of directories and symlinks (or hardlinks if --hard is passed) to mirror a target
  -v, --verbose      Describe any changes to be made to the filesystem
  -n, --dry-run      Don't modify the filesystem
  -h, --help         Print help (see more with '--help')
  -V, --version      Print version
```

### `slinky-ln` Examples

All examples have the `--verbose` flag for illustration purposes.

Create a symlink to the given file, within the current directory, reusing the
basename of the target file:

```console
$ slinky-ln -v ./foo/bar
create symlink: ./bar -> foo/bar
```

Create a hardlink instead of a symlink (the log shows the canonical path of the
target):

```console
$ slinky-ln -vH ./foo/bar
create hardlink: ./bar -> /home/user/foo/bar
```

Write the target text exactly as given, even if it does not resolve to a file:

```console
$ slinky-ln -v --literal ../config/settings.toml ./settings.toml
create symlink: ./settings.toml -> ../config/settings.toml
```

Mirror a directory as a tree of symlinks:

```console
$ slinky-ln -vT /data/snapshot ./snapshot-mirror
create symlink tree: ./snapshot-mirror -> /data/snapshot
```

## License

Copyright 2026 Dylan Forbes. MIT license.
