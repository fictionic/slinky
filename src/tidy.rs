use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Result;
use std::path::{Component, Path, PathBuf};

use crate::cli::TidyTargetOpts;
use crate::path::symlink_parent;
use crate::semantic_path::{SemanticPath, SemanticPathSegment};

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
    pub fn new(walker_root_path: PathBuf) -> Result<Self> {
        let (origin_anchor_path, origin_anchor_depth) = get_origin_anchor(&walker_root_path)?;
        Ok(PathTidier {
            walker_root_path,
            origin_anchor_path,
            origin_anchor_depth,
        })
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
        root_path_abs = std::path::absolute(walker_root_path)?;
        symlink_parent(&root_path_abs)
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
    // first parse into a structure that encodes the kernel semantics.
    let parsed = SemanticPath::try_from(target_path).expect("target_path must not be empty"); // we preemptively skip empty links

    // then collapse in-and-out dot-dot segments where appropriate.
    let segments_base_dir = if parsed.is_absolute {
        PathBuf::from("/")
    } else {
        symlink_parent(origin_path).to_path_buf()
    };
    let mut segments =
        collapse_in_and_out(parsed.segments, segments_base_dir, opts.lexical_in_and_out);

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

    // then assemble into a new semantic path
    let final_parsed = SemanticPath {
        is_absolute: parsed.is_absolute,
        segments,
        must_be_directory,
    };

    // then render
    final_parsed.to_os_string()
}

fn collapse_in_and_out(
    segments: Vec<SemanticPathSegment>,
    segments_base_dir: PathBuf,
    lexical: bool,
) -> Vec<SemanticPathSegment> {
    let is_absolute = segments_base_dir == Path::new("/");
    if lexical {
        collapse_in_and_out_impl(segments, is_absolute, |_| true)
    } else {
        collapse_in_and_out_impl(segments, is_absolute, make_dir_checker(segments_base_dir))
    }
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

fn make_dir_checker(segments_base_dir: PathBuf) -> impl Fn(&[SemanticPathSegment]) -> bool {
    use SemanticPathSegment::*;
    move |prefix_segments| {
        let mut probe_path = segments_base_dir.clone();
        for token in prefix_segments {
            match token {
                Normal(n) => probe_path.push(n),
                DotDot => probe_path.push(".."),
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

/// Like `std::path::absolute`, but tidies the result, using default settings.
pub fn tidied_absolute_path(path: &Path) -> Result<PathBuf> {
    let path_abs = std::path::absolute(path)?;
    let parsed = SemanticPath::try_from(path_abs.as_path()).expect("path_abs must not be empty"); // absolute() already errors on an empty path
    let segments = collapse_in_and_out(parsed.segments, PathBuf::from("/"), false);
    Ok(PathBuf::from(SemanticPath {
        segments,
        is_absolute: true,
        must_be_directory: parsed.must_be_directory,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

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
        let is_real_dir = make_dir_checker(dir.path().to_path_buf());
        assert!(is_real_dir(&segs(&["d"])));
    }

    #[test]
    fn dir_checker_relative_symlink_to_dir_is_false() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("real")).unwrap();
        symlink("real", dir.path().join("slink")).unwrap();
        let is_real_dir = make_dir_checker(dir.path().to_path_buf());
        // lstat, not stat: a symlink to a directory is not itself a directory
        assert!(!is_real_dir(&segs(&["slink"])));
    }

    #[test]
    fn dir_checker_nonexistent_is_false() {
        let dir = tempdir().unwrap();
        let is_real_dir = make_dir_checker(dir.path().to_path_buf());
        assert!(!is_real_dir(&segs(&["nope"])));
    }

    #[test]
    fn dir_checker_absolute_base_is_root() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("d")).unwrap();
        // an absolute prefix probes from "/"
        let is_real_dir = make_dir_checker(PathBuf::from("/"));
        let mut prefix = normal_segments(dir.path());
        prefix.push(SemanticPathSegment::Normal(OsString::from("d")));
        assert!(is_real_dir(&prefix));
    }

    // tidied_absolute_path (filesystem seam: absolute from cwd + lstat probes)

    #[test]
    fn tidied_absolute_path_collapses_parent_of_real_dir() {
        let dir = tempdir().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        fs::create_dir(base.join("d")).unwrap();
        assert_eq!(
            tidied_absolute_path(&base.join("d/../f")).unwrap(),
            base.join("f")
        );
    }

    #[test]
    fn tidied_absolute_path_keeps_parent_of_symlink() {
        let dir = tempdir().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        fs::create_dir(base.join("real")).unwrap();
        symlink("real", base.join("slink")).unwrap();
        // slink/.. climbs from real's parent, which is not the same as dropping slink
        assert_eq!(
            tidied_absolute_path(&base.join("slink/../f")).unwrap(),
            base.join("slink/../f")
        );
    }

    #[test]
    fn tidied_absolute_path_keeps_parent_of_missing_component() {
        let dir = tempdir().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        assert_eq!(
            tidied_absolute_path(&base.join("nope/../f")).unwrap(),
            base.join("nope/../f")
        );
    }

    #[test]
    fn tidied_absolute_path_keeps_trailing_slash() {
        let dir = tempdir().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        fs::create_dir(base.join("d")).unwrap();
        let mut path = base.join("d").into_os_string();
        path.push("/");
        assert_eq!(
            tidied_absolute_path(Path::new(&path)).unwrap(),
            PathBuf::from(path)
        );
    }

    #[test]
    fn tidied_absolute_path_root_parent_is_root() {
        assert_eq!(
            tidied_absolute_path(Path::new("/..")).unwrap(),
            Path::new("/")
        );
    }

    #[test]
    fn tidied_absolute_path_relative_resolves_from_cwd() {
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(
            tidied_absolute_path(Path::new("f")).unwrap(),
            cwd.join("f")
        );
    }

    #[test]
    fn tidied_absolute_path_leading_parent_climbs_out_of_cwd() {
        // cwd is physical, so each component it contributes is a real dir
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(
            tidied_absolute_path(Path::new("../f")).unwrap(),
            cwd.parent().unwrap().join("f")
        );
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
