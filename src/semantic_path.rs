use std::{
    ffi::{OsStr, OsString},
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

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
#[derive(Debug, PartialEq, Clone)]
pub enum SemanticPathSegment {
    DotDot,
    Normal(OsString),
}

pub struct SemanticPath {
    /// The semantically meaningful inter-separator components of the path.
    pub segments: Vec<SemanticPathSegment>,
    /// Whether the path starts with a `/`.
    pub is_absolute: bool,
    /// Whether the path contains an assertion that it resolves to a directory.
    pub must_be_directory: bool, // trailing slash, dot, or dot-dot
}

#[derive(Debug)]
pub struct EmptyPathError;
impl std::error::Error for EmptyPathError {}

impl std::fmt::Display for EmptyPathError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("empty path")
    }
}

impl TryFrom<&Path> for SemanticPath {
    type Error = EmptyPathError;

    fn try_from(path: &Path) -> Result<Self, Self::Error> {
        if path.as_os_str().is_empty() {
            // refuse to represent the empty path.
            return Err(EmptyPathError);
        }
        Ok(parse_tokens(tokenize_path(path)))
    }
}

impl From<SemanticPath> for PathBuf {
    fn from(path: SemanticPath) -> Self {
        PathBuf::from(path.to_os_string())
    }
}

impl SemanticPath {
    /// Returns `(parent_dir_path, entry_name)`, where `entry_name` is the canonical name (i.e. not
    /// '.' or '..') of the directory entry that the path refers to, and `parent_dir_path` is the
    /// path to the directory containing the entry. If this information is not knowable without
    /// accessing the filesystem, returns `None`.
    ///
    /// When a path ends in `/`, `.`, or `..`, it might not refer to a directory entry via its
    /// canonical name. If the final named component in such a path refers to a symlink, then it
    /// will be dereferenced in virtue of being followed by a path separator, and thus the path
    /// actually refers through the symlink. In this case there is no way to know what directory
    /// entry is referred to.
    pub fn split_canonical_entry_name(&self) -> Option<(SemanticPath, &OsStr)> {
        let (last, rest) = self.segments.split_last()?;
        match last {
            SemanticPathSegment::Normal(entry_name) if !self.must_be_directory => Some((
                SemanticPath {
                    segments: rest.to_vec(),
                    is_absolute: self.is_absolute,
                    must_be_directory: true,
                },
                entry_name,
            )),
            _ => None,
        }
    }

    pub fn to_path_buf(&self) -> PathBuf {
        PathBuf::from(self.to_os_string())
    }

    pub fn to_os_string(&self) -> OsString {
        let mut bytes: Vec<u8> = Vec::new();

        if self.is_absolute {
            bytes.push(b'/');
        }

        let mut first = true;
        for segment in &self.segments {
            if !first {
                bytes.push(b'/');
            }
            first = false;
            match segment {
                SemanticPathSegment::DotDot => bytes.extend_from_slice(b".."),
                SemanticPathSegment::Normal(n) => bytes.extend_from_slice(n.as_bytes()),
            }
        }

        // an empty relative target (e.g. "." or a fully folded "foo/..") renders to "."
        if bytes.is_empty() {
            return OsString::from(".");
        }

        // re-attach a directory assertion that folding may have erased.
        // guarded on non-empty segments so absolute root stays "/", never "//".
        if self.must_be_directory && !self.segments.is_empty() {
            bytes.push(b'/');
        }

        OsString::from_vec(bytes)
    }
}

fn parse_tokens(tokens: Vec<PathSegment>) -> SemanticPath {
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
    SemanticPath {
        segments,
        is_absolute,
        must_be_directory,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        SemanticPath {
            is_absolute,
            segments,
            must_be_directory,
        }
        .to_os_string()
    }

    // try_from

    #[test]
    fn try_from_nonempty_path_succeeds() {
        let result = SemanticPath::try_from(Path::new("foo/../bar/"));
        assert!(result.is_ok());
        let path = result.unwrap();
        assert_eq!(path.segments, segs(&["foo", "..", "bar"]));
        assert!(path.must_be_directory);
        assert!(!path.is_absolute);
    }

    #[test]
    fn try_from_empty_path_fails() {
        assert!(SemanticPath::try_from(Path::new("")).is_err());
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

    // to_os_string

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
}
