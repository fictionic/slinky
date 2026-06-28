# slinky
A utility for wrangling symbolic links on Unix systems.

## Why
Symlinks are great, but if you're like me, you occasionally find yourself
needing to "refactor" a large number of them at once. For example, you might
want to:

- Convert absolute symlinks to relative symlinks
- Canonicalize the target filesystem paths within symlinks

These tasks would be a pain to do with a custom shell script. The classic
[symlinks](https://github.com/brandt/symlinks) utility does handle them;
however, I found its features limited for my purposes. For example, imagine you
want to:

- Convert a bunch of relative symlinks to absolute symlinks
- Convert a symlink of a directory with a directory containing symlinks of the
  directory's contents
- Replace all symlinks within a given directory with hardlinks to their targets
  (or actual file copies of their targets)
- Given a bunch of symlinks that point into a given directory, rename that
  directory without breaking the symlinks

`slinky` can do all of these things.

> BONUS! Slinky also comes with a binary called `slinky-ln`, which offers most
> of the same behavior of classic `ln` but with a more clear CLI. See below.

(Obligatory "written in Rust", "blazing-fast", etc.)

## Installation

### From source

```sh
$ cargo build --release
```
(binaries land in target/release)

### Arch Linux

A `PKGBUILD` is included:

```sh
$ makepkg -si
```

It installs both binaries along with shell completions and man pages.
Shell completions (bash/fish/zsh) and man pages are generated at build time
into the `generate/` directory.

## Usage

```
Usage: slinky [OPTIONS] [PATH] <COMMAND>

Commands:
  list                 List symlinks [aliases: ls]
  to-relative          Convert absolute symlinks to relative symlinks. Fails on dangling symlinks
  to-absolute          Convert relative symlinks to absolute symlinks. Fails on dangling symlinks
  tidy                 Lexically tidy the target path (e.g., remove redundant `..` or `.`)
  edit-target          Edit the target string of symlinks by replacing regex matches
  to-hardlink          Convert symlinks to hardlinks. Fails on dangling symlinks, symlinks to directories, and cross-device symlinks
  to-tree              Convert a directory symlink into a directory tree of symlinks to files. Fails on dangling symlinks
  replace-with-target  Move the target to the symlink's location. Fails on dangling symlinks
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
```sh
$ slinky -r ./dir list --status
attached: ./foo/bar -> ../../baz
dangling: ./qux -> ../../doesnt-exist
```

Replace every instance of `/old/path` with `/new/path` in absolute symlinks,
and be verbose:
```sh
$ slinky -v edit-target '/old/path' '/new/path' --replace-all --verbose
edit-target: ./foo/bar -> (/mnt/foo/old/path/bar => /mnt/foo/new/path/bar)
edit-target: ./foo/baz -> (/mnt/foo/old/path/baz => /mnt/foo/new/path/baz)
```

Replace a directory symlink with a tree of symlinks:
```sh
$ slinky to-tree ./foo
```
(Note that this will create absolute symlinks. If you want relative links, you'll have
to convert them as a second step.)

Run an arbitrary command against each symlink using `$SHELL` (`$1` is bound to
the link origin, and `$2` is bound to the link target):
```sh
$ slinky exec 'echo "$1 points to $2"'
./foo/bar points to ../../baz
```

## Bonus: Create Symlinks

I can never remember how to use the `ln` command. I find its interface, and the
explanation in its man page, to be quite confusing. (Why does its invocation
have four forms???)

To address this, I created an alternative to `ln` called `slinky-ln`, which
has most of the smae features, plus some bonus ones, and an improved interface
and manual.

### `slinky-ln` Usage

```
Usage: slinky-ln [OPTIONS] <TARGET> [ORIGIN]

Arguments:
  <TARGET>  The path that the link will point to
  [ORIGIN]  The path where the link will live. If a directory is provided, the link will be created inside that directory with the same basename as the target [default: .]

Options:
  -f, --force           Force creation of the link by overwriting existing files. Will not overwrite directories
  -b, --absolute        Transform the target string into an absolute path to the target, if it exists
  -r, --relative        Transform the target string into a relative path to the target, if it exists
  -L, --dereference     Dereference the target file if it is a symbolic link
      --allow-dangling  Allow creation of dangling symlinks
  -H, --hard            Create a hardlink instead of a symlink
  -T, --tree            Create a tree of directories and symlinks (or hardlinks if --hard is passed) to mirror a target
  -v, --verbose         Describe any changes to be made to the filesystem
  -n, --dry-run         Don't modify the filesystem
  -h, --help            Print help
  -V, --version         Print version
```

### `slinky-ln` Examples

All examples have the `--verbose` flag for illustration purposes.

Create a symlink to the given file, within the current directory, reusing the
basename of the target file:
```sh
$ slinky-ln -v ./foo/bar
create symlink: bar -> ../../foo/bar
```

Create a hardlink instead of a symlink:
```sh
$ slinky-ln -vH ./foo/bar
create hardlink: bar -> ../../foo/bar
```

Mirror a directory as a tree of symlinks:
```sh
$ slinky-ln -vT /data/snapshot ./snapshot-mirror
create symlink tree: ./snapshot-mirror -> /data/snapshot
```
(Ideally I'd update this to print each link created.)

## License

MIT
