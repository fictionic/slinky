use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Result;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};

use crate::cli::TidyTargetOpts;
use crate::path::{get_absolute_origin_path, get_symlink_parent};

// encapsulating some per-run state in a class so it doesn't have to be
// computed per-link. actual tidying functionality is pure.
pub struct PathTidier {
    // the root of the fs walk, relative to CWD.
    // may be a symlink (if PATH points to a symlink).
    walker_root_path: PathBuf,
    // the canonical path to the directory containing all walked origin paths.
    origin_anchor_path: PathBuf,
    // the depth of origin_anchor_path, precomputed.
    origin_anchor_depth: usize,
}

impl PathTidier {
    pub fn new(walker_root_path: PathBuf) -> Self {
        let (origin_anchor_path, origin_anchor_depth) =
            get_origin_anchor(&walker_root_path).expect("PATH should exist");
        PathTidier {
            walker_root_path,
            origin_anchor_path,
            origin_anchor_depth,
        }
    }

    pub fn tidy(&self, target_path: &Path, origin_path: &Path, opts: &TidyTargetOpts) -> OsString {
        tidy_target_path(
            target_path,
            origin_path,
            &self.walker_root_path,
            &self.origin_anchor_path,
            self.origin_anchor_depth,
            opts,
        )
    }
}

fn get_origin_anchor(walker_root_path: &Path) -> Result<(PathBuf, usize)> {
    let root_is_symlink = fs::symlink_metadata(walker_root_path)?.is_symlink();
    let root_path_abs;
    let anchor_path = if root_is_symlink {
        root_path_abs = get_absolute_origin_path(walker_root_path);
        get_symlink_parent(&root_path_abs)
    } else {
        walker_root_path
    };
    let anchor_path = fs::canonicalize(anchor_path)?;
    let anchor_depth = anchor_path
        .components()
        // strip the leading Component::RootDir
        .filter(|c| matches!(c, Component::Normal(_)))
        .count();
    Ok((anchor_path, anchor_depth))
}

fn tidy_target_path(
    target_path: &Path,
    origin_path: &Path,
    walker_root_path: &Path,
    origin_anchor_path: &Path,
    origin_anchor_depth: usize,
    opts: &TidyTargetOpts,
) -> OsString {
    let tokens = tokenize_path(target_path);

    // first parse into a structure that encodes the semantics of the link.
    // this strips empty segments and redundant dot segments.
    let parsed = parse_tokens(tokens);

    // then collapse in-and-out dot-dot segments where appropriate.
    let mut segments = collapse_in_and_out(
        parsed.segments,
        parsed.is_absolute,
        origin_path,
        opts.lexical_in_and_out,
    );

    let origin_walker_descent = origin_path
        .strip_prefix(walker_root_path)
        .expect("walker_root_path should be a prefix of origin_path");

    // then strip dot-dot segments that redundantly resolve to the fs root,
    // within relative symlinks (collapse_in_and_out already stripped them
    // for absolute symlinks)
    if !parsed.is_absolute {
        segments = strip_relative_root_parent(segments, origin_walker_descent, origin_anchor_depth);
    };

    // then collapse out-and-in dot-dot segments, if so instructed
    if opts.collapse_out_and_in {
        segments = collapse_out_and_in(segments, origin_walker_descent, origin_anchor_path);
    };

    // then strip trailing slash, if so instructed
    let must_be_directory = if opts.strip_trailing_slash {
        false
    } else {
        parsed.must_be_directory
    };

    // then render
    let final_parsed = ParsedPath {
        is_absolute: parsed.is_absolute,
        segments,
        must_be_directory,
    };
    render_to_os_string(final_parsed)
}

// any lexical inter-slash component of a path
#[derive(Debug, PartialEq)]
enum PathSegment {
    Empty,
    Dot,
    DotDot,
    Normal(OsString),
}

fn tokenize_path(path: &Path) -> Vec<PathSegment> {
    let bytes = path.as_os_str().as_bytes();

    let mut tokens = Vec::new();
    for piece in bytes.split(|&b| b == b'/') {
        match piece {
            b"" => {
                tokens.push(PathSegment::Empty);
            }
            b"." => {
                tokens.push(PathSegment::Dot);
            }
            b".." => {
                tokens.push(PathSegment::DotDot);
            }
            normal => {
                tokens.push(PathSegment::Normal(OsStr::from_bytes(normal).to_owned()));
            }
        }
    }
    tokens
}

// only the semantics-affecting inter-slash components of paths
#[derive(Debug, PartialEq)]
enum SemanticPathSegment {
    DotDot,
    Normal(OsString),
}

struct ParsedPath {
    segments: Vec<SemanticPathSegment>,
    is_absolute: bool,       // leading slash
    must_be_directory: bool, // trailing slash, dot, or dot-dot
}

fn parse_tokens(tokens: Vec<PathSegment>) -> ParsedPath {
    // first token is empty -> starts with a / -> absolute
    let is_absolute = matches!(tokens.first(), Some(PathSegment::Empty),);
    // last token is empty/dot/dot-dot -> ends with a / or . or .. -> assertion
    // that the target is a directory (enforced by the kernel)
    let must_be_directory = matches!(
        tokens.last(),
        Some(PathSegment::Empty | PathSegment::Dot | PathSegment::DotDot),
    );
    let segments = tokens
        .into_iter()
        .filter_map(|seg| match seg {
            // empty- and dot-segments within the path are redundant; drop them.
            // we've already captured all we need from the leading and trailing
            // segments.
            PathSegment::Empty | PathSegment::Dot => None,
            // keep everything else for now
            PathSegment::DotDot => Some(SemanticPathSegment::DotDot),
            PathSegment::Normal(n) => Some(SemanticPathSegment::Normal(n)),
        })
        .collect();
    ParsedPath {
        segments,
        is_absolute,
        must_be_directory,
    }
}

fn collapse_in_and_out(
    segments: Vec<SemanticPathSegment>,
    is_absolute: bool,
    origin_path: &Path,
    lexical: bool,
) -> Vec<SemanticPathSegment> {
    let is_real_dir = make_dir_checker(origin_path, is_absolute, lexical);

    collapse_in_and_out_impl(segments, is_absolute, is_real_dir)
}

fn collapse_in_and_out_impl(
    segments: Vec<SemanticPathSegment>,
    is_absolute: bool,
    is_real_dir: impl Fn(&[SemanticPathSegment]) -> bool,
) -> Vec<SemanticPathSegment> {
    use SemanticPathSegment::*;

    let mut collapsed: Vec<SemanticPathSegment> = Vec::with_capacity(segments.len());

    for segment in segments {
        match segment {
            Normal(n) => collapsed.push(Normal(n)),
            DotDot => match collapsed.last() {
                // fold .. up into its parent if its parent is a real dir
                Some(Normal(_)) if is_real_dir(&collapsed) => {
                    collapsed.pop();
                }
                // keep .. when its parent is not confirmed as a real dir
                Some(Normal(_)) => collapsed.push(DotDot),
                // can't fold .. into another ..
                Some(DotDot) => collapsed.push(DotDot),
                // fold /.. into / (root dir is its own parent)
                None if is_absolute => {}
                // relative link that starts with a ..
                None => collapsed.push(DotDot),
            },
        }
    }

    collapsed
}

fn make_dir_checker(
    origin_path: &Path,
    is_absolute: bool,
    lexical: bool,
) -> impl Fn(&[SemanticPathSegment]) -> bool {
    // TODO: skip this when lexical is true?
    let base = if is_absolute {
        PathBuf::from("/")
    } else {
        get_symlink_parent(origin_path).to_path_buf()
    };

    move |prefix_segments| {
        if lexical {
            return true;
        }

        let mut probe_path = base.clone();
        for token in prefix_segments {
            match token {
                SemanticPathSegment::Normal(n) => probe_path.push(n),
                SemanticPathSegment::DotDot => probe_path.push(".."),
            }
        }

        fs::symlink_metadata(&probe_path)
            .map(|m| m.is_dir())
            .unwrap_or(false)
    }
}

fn strip_relative_root_parent(
    mut segments: Vec<SemanticPathSegment>,
    origin_walker_descent: &Path,
    origin_anchor_depth: usize,
) -> Vec<SemanticPathSegment> {
    // leading dot-dots beyond the link's own depth climb past the root, which
    // absorbs them (the root is its own parent). the first `origin_depth` of
    // them are real climbs up to the root, so they stay.
    let leading_dotdots = segments
        .iter()
        .take_while(|s| matches!(s, SemanticPathSegment::DotDot))
        .count();
    let origin_descent_dirs = origin_walker_descent.components().count().saturating_sub(1);
    let origin_depth = origin_anchor_depth + origin_descent_dirs;
    let redundant_dotdots = leading_dotdots.saturating_sub(origin_depth);

    segments.drain(0..redundant_dotdots);
    segments
}

fn collapse_out_and_in(
    mut segments: Vec<SemanticPathSegment>,
    origin_walker_descent: &Path,
    origin_anchor_path: &Path,
) -> Vec<SemanticPathSegment> {
    let run_length = segments
        .iter()
        .take_while(|s| matches!(s, SemanticPathSegment::DotDot))
        .count();
    if run_length == 0 {
        // note that this means we only act on relative symlinks
        return segments;
    }

    let canonical_origin_parent_path =
        origin_anchor_path.join(origin_walker_descent.parent().unwrap_or(Path::new("")));

    // climb up to run_length ancestors from the link's own directory,
    // recording each ancestor's name and whether it is a real directory.
    // ancestors[0] is the link's directory; the last entry is the highest
    // ancestor the dot-dot run climbs to.
    let mut ancestor = canonical_origin_parent_path.as_path();
    let mut ancestors: Vec<&OsStr> = Vec::with_capacity(run_length);
    for _ in 0..run_length {
        let ascent_name = ancestor
            .file_name()
            // we already stripped redundant root-parent climbs in
            // collapse_in_and_out() (absolute) and in
            // strip_relative_root_parent() (relative)
            .expect("ancestor should not be the filesystem root");
        ancestors.push(ascent_name);
        ancestor = match ancestor.parent() {
            Some(parent) => parent,
            None => break,
        };
    }

    // greedily cancel round-trips from the outside in: the first descent
    // segment pairs with the highest ancestor's name, the next with the ancestor
    // below it, and so on. stop at the first name mismatch or non-real pivot;
    // whatever prefix matched is a genuine no-op we can drop.
    let mut cancelled = 0;
    while cancelled < ancestors.len() && run_length + cancelled < segments.len() {
        let ascent_name = ancestors[ancestors.len() - 1 - cancelled];
        let descends_back = matches!(
            &segments[run_length + cancelled],
            SemanticPathSegment::Normal(descent_name) if descent_name.as_os_str() == ascent_name,
        );
        if !descends_back {
            break;
        }
        cancelled += 1;
    }

    // drop the matched descent segments first (higher indices) so the leading
    // dot-dot indices stay valid, then the dot-dots they cancelled with.
    segments.drain(run_length..run_length + cancelled);
    segments.drain(0..cancelled);
    segments
}

fn render_to_os_string(parsed: ParsedPath) -> OsString {
    use SemanticPathSegment::*;

    let mut bytes: Vec<u8> = Vec::new();

    if parsed.is_absolute {
        bytes.push(b'/');
    }

    let mut first = true;
    for segment in &parsed.segments {
        if !first {
            bytes.push(b'/');
        }
        first = false;
        match segment {
            DotDot => bytes.extend_from_slice(b".."),
            Normal(n) => bytes.extend_from_slice(n.as_bytes()),
        }
    }

    // an empty relative target (e.g. "." or a fully folded "foo/..") renders to "."
    if bytes.is_empty() {
        return OsString::from(".");
    }

    // re-attach a directory assertion that folding may have erased.
    // guarded on non-empty segments so absolute root stays "/", never "//".
    if parsed.must_be_directory && !parsed.segments.is_empty() {
        bytes.push(b'/');
    }

    OsString::from_vec(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    // "" -> Empty, "." -> Dot, ".." -> DotDot, anything else -> Normal
    fn toks(parts: &[&str]) -> Vec<PathSegment> {
        parts
            .iter()
            .map(|&p| match p {
                "" => PathSegment::Empty,
                "." => PathSegment::Dot,
                ".." => PathSegment::DotDot,
                n => PathSegment::Normal(OsString::from(n)),
            })
            .collect()
    }

    // ".." -> DotDot, anything else -> Normal
    fn segs(parts: &[&str]) -> Vec<SemanticPathSegment> {
        parts
            .iter()
            .map(|&p| match p {
                ".." => SemanticPathSegment::DotDot,
                n => SemanticPathSegment::Normal(OsString::from(n)),
            })
            .collect()
    }

    fn rendered(
        is_absolute: bool,
        segments: Vec<SemanticPathSegment>,
        must_be_directory: bool,
    ) -> OsString {
        render_to_os_string(ParsedPath {
            is_absolute,
            segments,
            must_be_directory,
        })
    }

    // tokenize_path

    #[test]
    fn tokenize_relative() {
        assert_eq!(tokenize_path(Path::new("a/b")), toks(&["a", "b"]));
    }

    #[test]
    fn tokenize_leading_and_trailing_slash() {
        assert_eq!(tokenize_path(Path::new("/a")), toks(&["", "a"]));
        assert_eq!(tokenize_path(Path::new("a/")), toks(&["a", ""]));
    }

    #[test]
    fn tokenize_interior_empty_and_dots() {
        assert_eq!(tokenize_path(Path::new("a//b")), toks(&["a", "", "b"]));
        assert_eq!(
            tokenize_path(Path::new("/a/./b/")),
            toks(&["", "a", ".", "b", ""])
        );
    }

    #[test]
    fn tokenize_bare_dot_and_dotdot() {
        assert_eq!(tokenize_path(Path::new(".")), toks(&["."]));
        assert_eq!(tokenize_path(Path::new("..")), toks(&[".."]));
    }

    #[test]
    fn tokenize_empty_string() {
        assert_eq!(tokenize_path(Path::new("")), toks(&[""]));
    }

    #[test]
    fn tokenize_non_utf8_segment_is_byte_exact() {
        let path = Path::new(OsStr::from_bytes(b"a/\xff/b"));
        let expected = vec![
            PathSegment::Normal(OsString::from("a")),
            PathSegment::Normal(OsStr::from_bytes(b"\xff").to_owned()),
            PathSegment::Normal(OsString::from("b")),
        ];
        assert_eq!(tokenize_path(path), expected);
    }

    // parse_tokens

    #[test]
    fn parse_absolute_dir_drops_empties_and_dots() {
        let parsed = parse_tokens(toks(&["", "a", ".", "b", ""]));
        assert!(parsed.is_absolute);
        assert!(parsed.must_be_directory);
        assert_eq!(parsed.segments, segs(&["a", "b"]));
    }

    #[test]
    fn parse_relative_keeps_dotdot() {
        let parsed = parse_tokens(toks(&["a", "..", "b"]));
        assert!(!parsed.is_absolute);
        assert!(!parsed.must_be_directory);
        assert_eq!(parsed.segments, segs(&["a", "..", "b"]));
    }

    #[test]
    fn parse_trailing_dotdot_asserts_directory() {
        let parsed = parse_tokens(toks(&["a", ".."]));
        assert!(parsed.must_be_directory);
        assert_eq!(parsed.segments, segs(&["a", ".."]));
    }

    #[test]
    fn parse_empty_string_is_absolute_dir() {
        let parsed = parse_tokens(toks(&[""]));
        assert!(parsed.is_absolute);
        assert!(parsed.must_be_directory);
        assert_eq!(parsed.segments, segs(&[]));
    }

    // collapse_in_and_out_impl (the fs probe is injected)

    #[test]
    fn collapse_in_folds_when_parent_is_real_dir() {
        let out = collapse_in_and_out_impl(segs(&["a", "..", "b"]), false, |_| true);
        assert_eq!(out, segs(&["b"]));
    }

    #[test]
    fn collapse_in_keeps_when_parent_not_real_dir() {
        let out = collapse_in_and_out_impl(segs(&["a", "..", "b"]), false, |_| false);
        assert_eq!(out, segs(&["a", "..", "b"]));
    }

    #[test]
    fn collapse_in_absolute_root_absorbs_leading_dotdot() {
        let out = collapse_in_and_out_impl(segs(&["..", ".."]), true, |_| true);
        assert_eq!(out, segs(&[]));
    }

    #[test]
    fn collapse_in_relative_keeps_leading_dotdot() {
        let out = collapse_in_and_out_impl(segs(&["..", ".."]), false, |_| true);
        assert_eq!(out, segs(&["..", ".."]));
    }

    #[test]
    fn collapse_in_probes_collapsed_prefix() {
        // fold a/.. (prefix ["a"], len 1) but then keep the trailing .. because
        // there is no real dir left to fold it into.
        let out = collapse_in_and_out_impl(
            segs(&["a", "..", ".."]),
            false,
            |prefix: &[SemanticPathSegment]| prefix.len() == 1,
        );
        assert_eq!(out, segs(&[".."]));
    }

    // strip_relative_root_parent (origin_depth driven by descent + anchor_depth)

    #[test]
    fn strip_keeps_dotdots_within_depth() {
        // descent "a/b/link" -> 2 descent dirs; anchor_depth 1 -> origin_depth 3.
        let out =
            strip_relative_root_parent(segs(&["..", "..", "..", "x"]), Path::new("a/b/link"), 1);
        assert_eq!(out, segs(&["..", "..", "..", "x"]));
    }

    #[test]
    fn strip_drops_dotdots_beyond_depth() {
        // origin_depth 3, four leading dot-dots -> one is redundant.
        let out = strip_relative_root_parent(
            segs(&["..", "..", "..", "..", "x"]),
            Path::new("a/b/link"),
            1,
        );
        assert_eq!(out, segs(&["..", "..", "..", "x"]));
    }

    #[test]
    fn strip_depth_zero_drops_all_leading_dotdots() {
        // descent "link" -> 0 descent dirs; anchor_depth 0 -> origin_depth 0.
        let out = strip_relative_root_parent(segs(&["..", "..", "x"]), Path::new("link"), 0);
        assert_eq!(out, segs(&["x"]));
    }

    #[test]
    fn strip_leaves_interior_dotdots() {
        let out = strip_relative_root_parent(segs(&["x", "..", ".."]), Path::new("a/b/link"), 1);
        assert_eq!(out, segs(&["x", "..", ".."]));
    }

    // collapse_out_and_in (pure: ancestors derived lexically from anchor + descent)
    //
    // anchor "/p" + descent "src/a/link" -> link dir "/p/src/a",
    // so the climbed ancestor names (link dir first) are ["a", "src", "p"].

    fn anchor() -> &'static Path {
        Path::new("/p")
    }
    fn descent() -> &'static Path {
        Path::new("src/a/link")
    }

    #[test]
    fn collapse_out_full_cancel() {
        let out = collapse_out_and_in(segs(&["..", "..", "src", "a", "x"]), descent(), anchor());
        assert_eq!(out, segs(&["x"]));
    }

    #[test]
    fn collapse_out_partial_cancel_halts_on_mismatch() {
        let out = collapse_out_and_in(segs(&["..", "..", "src", "ZZ"]), descent(), anchor());
        assert_eq!(out, segs(&["..", "ZZ"]));
    }

    #[test]
    fn collapse_out_no_cancel_on_top_mismatch() {
        let out = collapse_out_and_in(segs(&["..", "..", "QQ", "a", "x"]), descent(), anchor());
        assert_eq!(out, segs(&["..", "..", "QQ", "a", "x"]));
    }

    #[test]
    fn collapse_out_no_leading_dotdot_is_noop() {
        let out = collapse_out_and_in(segs(&["a", "b"]), descent(), anchor());
        assert_eq!(out, segs(&["a", "b"]));
    }

    #[test]
    #[should_panic(expected = "filesystem root")]
    fn collapse_out_panics_when_run_climbs_past_root() {
        // caller must strip overshooting dot-dots first; feeding an unstripped
        // run deeper than the anchor climbs past "/" and trips the expect.
        collapse_out_and_in(segs(&["..", ".."]), Path::new("link"), Path::new("/p"));
    }

    // render_to_os_string

    #[test]
    fn render_absolute_root() {
        assert_eq!(rendered(true, segs(&[]), false), OsString::from("/"));
    }

    #[test]
    fn render_empty_relative_is_dot() {
        assert_eq!(rendered(false, segs(&[]), false), OsString::from("."));
    }

    #[test]
    fn render_absolute_with_segments() {
        assert_eq!(
            rendered(true, segs(&["a", "b"]), false),
            OsString::from("/a/b")
        );
    }

    #[test]
    fn render_reattaches_trailing_slash() {
        assert_eq!(rendered(false, segs(&["a"]), true), OsString::from("a/"));
        assert_eq!(rendered(true, segs(&["a"]), true), OsString::from("/a/"));
    }

    #[test]
    fn render_absolute_root_never_double_slashes() {
        assert_eq!(rendered(true, segs(&[]), true), OsString::from("/"));
    }

    #[test]
    fn render_dotdot() {
        assert_eq!(
            rendered(false, segs(&["..", "a"]), false),
            OsString::from("../a")
        );
    }

    #[test]
    fn render_non_utf8_is_byte_exact() {
        let segments = vec![SemanticPathSegment::Normal(
            OsStr::from_bytes(b"\xff").to_owned(),
        )];
        assert_eq!(
            rendered(false, segments, false),
            OsStr::from_bytes(b"\xff").to_owned()
        );
    }

    // the Normal components of a path, as descent segments
    fn normal_segments(path: &Path) -> Vec<SemanticPathSegment> {
        path.components()
            .filter_map(|c| match c {
                Component::Normal(n) => Some(SemanticPathSegment::Normal(n.to_owned())),
                _ => None,
            })
            .collect()
    }

    // make_dir_checker (filesystem seam: lstat semantics)

    #[test]
    fn dir_checker_relative_real_dir_is_true() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("d")).unwrap();
        let origin = dir.path().join("link");
        let is_real_dir = make_dir_checker(&origin, false, false);
        assert!(is_real_dir(&segs(&["d"])));
    }

    #[test]
    fn dir_checker_relative_symlink_to_dir_is_false() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("real")).unwrap();
        symlink("real", dir.path().join("slink")).unwrap();
        let origin = dir.path().join("link");
        let is_real_dir = make_dir_checker(&origin, false, false);
        // lstat, not stat: a symlink to a directory is not itself a directory
        assert!(!is_real_dir(&segs(&["slink"])));
    }

    #[test]
    fn dir_checker_nonexistent_is_false() {
        let dir = tempdir().unwrap();
        let origin = dir.path().join("link");
        let is_real_dir = make_dir_checker(&origin, false, false);
        assert!(!is_real_dir(&segs(&["nope"])));
    }

    #[test]
    fn dir_checker_lexical_never_probes() {
        let dir = tempdir().unwrap();
        let origin = dir.path().join("link");
        let is_real_dir = make_dir_checker(&origin, false, true);
        assert!(is_real_dir(&segs(&["nope"])));
    }

    #[test]
    fn dir_checker_absolute_base_is_root() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("d")).unwrap();
        // when is_absolute, the base is "/" and origin's parent is unused.
        let origin = PathBuf::from("/unused/link");
        let is_real_dir = make_dir_checker(&origin, true, false);
        let mut prefix = normal_segments(dir.path());
        prefix.push(SemanticPathSegment::Normal(OsString::from("d")));
        assert!(is_real_dir(&prefix));
    }

    // get_origin_anchor (filesystem seam: canonicalize + parent-anchoring)

    #[test]
    fn anchor_real_dir_root() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();
        let (anchor, depth) = get_origin_anchor(&sub).unwrap();
        let expected = fs::canonicalize(&sub).unwrap();
        assert_eq!(anchor, expected);
        let expected_depth = expected
            .components()
            .filter(|c| matches!(c, Component::Normal(_)))
            .count();
        assert_eq!(depth, expected_depth);
    }

    #[test]
    fn anchor_symlink_root_uses_parent() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("realdir")).unwrap();
        symlink("realdir", dir.path().join("linkroot")).unwrap();
        let (anchor, _) = get_origin_anchor(&dir.path().join("linkroot")).unwrap();
        // a symlink root anchors at its parent, not through the link's target
        assert_eq!(anchor, fs::canonicalize(dir.path()).unwrap());
    }

    #[test]
    fn anchor_resolves_prefix_symlink() {
        let dir = tempdir().unwrap();
        let realdir = dir.path().join("realdir");
        fs::create_dir(&realdir).unwrap();
        fs::create_dir(realdir.join("leaf")).unwrap();
        symlink("realdir", dir.path().join("plink")).unwrap();
        // the root leaf is a real dir, but the prefix component "plink" is a
        // symlink that canonicalize must resolve.
        let (anchor, _) = get_origin_anchor(&dir.path().join("plink").join("leaf")).unwrap();
        assert_eq!(anchor, fs::canonicalize(realdir.join("leaf")).unwrap());
    }
}
