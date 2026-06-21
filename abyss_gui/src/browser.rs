//! The Commander's map.
//!
//! A [`Location`] is one of three things: the list of drives, a real directory,
//! or a *virtual* position inside an archive. The last is the trick that lets a
//! user step into a `.tar.zst` as though it were a folder — the archive's flat
//! table of contents (from [`archive_engine::list`], read once and cached) is
//! re-projected into folder-by-folder navigation on demand. Nothing is ever
//! decompressed to disk to look inside.

use archive_engine::{Format, Listing};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Where the Commander currently stands.
#[derive(Clone)]
pub enum Location {
    /// The list of drive roots (`C:\`, `D:\`, …). The top of the world.
    Drives,
    /// A real directory on disk.
    Fs(PathBuf),
    /// A virtual position inside an archive at internal path `inner`
    /// (`""` = archive root, otherwise always ends in `/`).
    Archive {
        path: PathBuf,
        format: Format,
        listing: Arc<Listing>,
        inner: String,
    },
}

/// What a row represents — drives navigation glyph and behavior follow from it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowKind {
    Parent,
    Drive,
    Dir,
    Archive,
    File,
}

/// One line in the Commander's list.
#[derive(Clone)]
pub struct BrowseRow {
    pub name: String,
    pub kind: RowKind,
    pub size: Option<u64>,
}

/// The result of activating a row.
pub enum Activated {
    /// Move to a new location.
    Go(Location),
    /// Nothing navigational — caller should just select the row.
    Stay,
    /// Opening an archive failed.
    Error(String),
}

impl Location {
    /// A human-readable breadcrumb for the location bar.
    pub fn label(&self) -> String {
        match self {
            Location::Drives => "This PC".to_string(),
            Location::Fs(dir) => dir.display().to_string(),
            Location::Archive { path, inner, .. } => {
                let archive = path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if inner.is_empty() {
                    format!("{archive}  ›")
                } else {
                    format!("{archive}  ›  {}", inner.trim_end_matches('/').replace('/', "  ›  "))
                }
            }
        }
    }

    /// Whether a "go up" step is possible from here.
    pub fn can_up(&self) -> bool {
        !matches!(self, Location::Drives)
    }

    /// Whether we are currently peering inside an archive.
    pub fn in_archive(&self) -> bool {
        matches!(self, Location::Archive { .. })
    }
}

/// The starting point: the user's home, else the current dir, else the drives.
pub fn initial() -> Location {
    if let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        if home.is_dir() {
            return Location::Fs(home);
        }
    }
    match std::env::current_dir() {
        Ok(dir) => Location::Fs(dir),
        Err(_) => Location::Drives,
    }
}

/// Compute the rows visible at `loc`.
pub fn rows_for(loc: &Location) -> Vec<BrowseRow> {
    match loc {
        Location::Drives => drive_rows(),
        Location::Fs(dir) => fs_rows(dir),
        Location::Archive { listing, inner, .. } => archive_rows(listing, inner),
    }
}

/// The location one step up, if any.
pub fn up(loc: &Location) -> Option<Location> {
    match loc {
        Location::Drives => None,
        Location::Fs(dir) => Some(match dir.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => Location::Fs(parent.to_path_buf()),
            _ => Location::Drives,
        }),
        Location::Archive { path, format, listing, inner } => {
            if inner.is_empty() {
                // Step back out of the archive into its containing folder.
                Some(match path.parent() {
                    Some(parent) if !parent.as_os_str().is_empty() => {
                        Location::Fs(parent.to_path_buf())
                    }
                    _ => Location::Drives,
                })
            } else {
                Some(Location::Archive {
                    path: path.clone(),
                    format: *format,
                    listing: listing.clone(),
                    inner: parent_inner(inner),
                })
            }
        }
    }
}

/// Activate a row at `loc` (a click / open).
pub fn activate(loc: &Location, row: &BrowseRow) -> Activated {
    use Activated::*;
    match row.kind {
        RowKind::Parent => up(loc).map(Go).unwrap_or(Stay),
        RowKind::Drive => Go(Location::Fs(PathBuf::from(&row.name))),
        RowKind::Dir => match loc {
            Location::Fs(dir) => Go(Location::Fs(dir.join(&row.name))),
            Location::Archive { path, format, listing, inner } => Go(Location::Archive {
                path: path.clone(),
                format: *format,
                listing: listing.clone(),
                inner: format!("{inner}{}/", row.name),
            }),
            Location::Drives => Stay,
        },
        RowKind::Archive => {
            let Location::Fs(dir) = loc else { return Stay };
            let target = dir.join(&row.name);
            match Format::from_path(&target) {
                // The Commander browses unsealed archives; an encrypted `.abyss`
                // needs a password, which the Extract tab handles.
                Some(format) => match archive_engine::list(&target, format, None) {
                    Ok(listing) => Go(Location::Archive {
                        path: target,
                        format,
                        listing: Arc::new(listing),
                        inner: String::new(),
                    }),
                    Err(e) => Error(format!("could not read archive: {e}")),
                },
                None => Stay,
            }
        }
        RowKind::File => Stay,
    }
}

// --- Row builders ----------------------------------------------------------

fn drive_rows() -> Vec<BrowseRow> {
    let mut rows = Vec::new();
    for letter in b'A'..=b'Z' {
        let root = format!("{}:\\", letter as char);
        if Path::new(&root).exists() {
            rows.push(BrowseRow { name: root, kind: RowKind::Drive, size: None });
        }
    }
    // Fall back to the unix root if no Windows drives were found.
    if rows.is_empty() && Path::new("/").exists() {
        rows.push(BrowseRow { name: "/".to_string(), kind: RowKind::Drive, size: None });
    }
    rows
}

fn fs_rows(dir: &Path) -> Vec<BrowseRow> {
    let mut rows = vec![BrowseRow { name: "..".to_string(), kind: RowKind::Parent, size: None }];

    let mut dirs: Vec<String> = Vec::new();
    let mut files: Vec<BrowseRow> = Vec::new();

    if let Ok(read) = fs::read_dir(dir) {
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                dirs.push(name);
            } else {
                let kind = if Format::from_path(Path::new(&name)).is_some() {
                    RowKind::Archive
                } else {
                    RowKind::File
                };
                let size = entry.metadata().ok().map(|m| m.len());
                files.push(BrowseRow { name, kind, size });
            }
        }
    }

    dirs.sort_by_key(|s| s.to_lowercase());
    files.sort_by_key(|r| r.name.to_lowercase());

    rows.extend(dirs.into_iter().map(|name| BrowseRow { name, kind: RowKind::Dir, size: None }));
    rows.extend(files);
    rows
}

fn archive_rows(listing: &Listing, inner: &str) -> Vec<BrowseRow> {
    let mut rows = vec![BrowseRow { name: "..".to_string(), kind: RowKind::Parent, size: None }];

    let mut dirs: BTreeSet<String> = BTreeSet::new();
    let mut files: Vec<BrowseRow> = Vec::new();

    for entry in &listing.entries {
        let full = entry.name.trim_end_matches('/');
        if !full.starts_with(inner) {
            continue;
        }
        let rest = &full[inner.len()..];
        if rest.is_empty() {
            continue;
        }
        match rest.find('/') {
            // Deeper than this level: the first segment is a subdirectory.
            Some(idx) => {
                dirs.insert(rest[..idx].to_string());
            }
            None if entry.is_dir => {
                dirs.insert(rest.to_string());
            }
            None => files.push(BrowseRow {
                name: rest.to_string(),
                kind: RowKind::File,
                size: entry.size,
            }),
        }
    }

    files.sort_by_key(|r| r.name.to_lowercase());

    rows.extend(dirs.into_iter().map(|name| BrowseRow { name, kind: RowKind::Dir, size: None }));
    rows.extend(files);
    rows
}

/// `"a/b/c/"` -> `"a/b/"`, `"a/"` -> `""`.
fn parent_inner(inner: &str) -> String {
    let trimmed = inner.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(idx) => trimmed[..=idx].to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archive_engine::{Codec, Container, Entry};

    fn sample() -> Listing {
        let entry = |name: &str, size: Option<u64>, is_dir: bool| Entry {
            name: name.to_string(),
            size,
            is_dir,
        };
        Listing {
            format: Format::new(Container::Tar, Codec::Zstd),
            entries: vec![
                entry("project/", Some(0), true),
                entry("project/src/", Some(0), true),
                entry("project/src/main.rs", Some(13), false),
                entry("project/README", Some(100), false),
                entry("notes.txt", Some(5), false),
            ],
            single_stream: false,
        }
    }

    fn names(rows: &[BrowseRow]) -> Vec<(&str, RowKind)> {
        rows.iter().map(|r| (r.name.as_str(), r.kind)).collect()
    }

    #[test]
    fn archive_root_shows_top_level_only() {
        let listing = sample();
        let rows = archive_rows(&listing, "");
        assert_eq!(
            names(&rows),
            vec![
                ("..", RowKind::Parent),
                ("project", RowKind::Dir),
                ("notes.txt", RowKind::File),
            ]
        );
    }

    #[test]
    fn archive_subdir_projects_children() {
        let listing = sample();
        let rows = archive_rows(&listing, "project/");
        assert_eq!(
            names(&rows),
            vec![
                ("..", RowKind::Parent),
                ("src", RowKind::Dir),
                ("README", RowKind::File),
            ]
        );
    }

    #[test]
    fn parent_inner_steps_up_one_level() {
        assert_eq!(parent_inner("project/src/"), "project/");
        assert_eq!(parent_inner("project/"), "");
        assert_eq!(parent_inner(""), "");
    }
}
