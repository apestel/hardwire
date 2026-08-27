use std::path::{Component, Path, PathBuf};

/// Lexically normalize a path (resolve `.` and `..` segments) without touching
/// the filesystem — safe for paths that do not exist yet (e.g. archive outputs).
/// On relative paths, `..` below the implicit root yields `..` (like a shell).
pub fn normalize_lexical(path: &Path) -> PathBuf {
    let absolute =
        path.components().next().is_some_and(|c| matches!(c, Component::RootDir));
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::RootDir => {
                if out.as_os_str().is_empty() {
                    out.push("/");
                }
            }
            Component::ParentDir => {
                let at_root = if absolute {
                    out.components().count() <= 1
                } else {
                    out.as_os_str().is_empty()
                };
                if at_root {
                    if !absolute {
                        out.push("..");
                    }
                    // absolute: stay at the root
                } else {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_strips_parent_segments() {
        assert_eq!(
            normalize_lexical(Path::new("/data/../etc/passwd")),
            PathBuf::from("/etc/passwd")
        );
        assert_eq!(
            normalize_lexical(Path::new("./data/./shared/../../x")),
            PathBuf::from("x")
        );
        // On relative paths, `..` below the implicit root stays visible.
        assert_eq!(
            normalize_lexical(Path::new("data/../../x")),
            PathBuf::from("../x")
        );
        // Absolute `..` at the root stays at the root.
        assert_eq!(normalize_lexical(Path::new("/../x")), PathBuf::from("/x"));
    }

    #[test]
    fn test_normalize_keeps_plain_paths() {
        assert_eq!(
            normalize_lexical(Path::new("/data/shared/file.txt")),
            PathBuf::from("/data/shared/file.txt")
        );
        assert_eq!(normalize_lexical(Path::new("data/file.txt")), PathBuf::from("data/file.txt"));
    }
}