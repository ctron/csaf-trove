//! Reusable tree comparisons that skip unchanged directory trees.

use anyhow::Result;
use git2::{Delta, ObjectType, Oid, Repository, Tree, TreeEntry};
use std::{
    collections::{BTreeSet, VecDeque},
    path::Path,
    sync::Arc,
};

/// Maximum tree pairs retained during one history scan.
const CACHE_ENTRIES: usize = 32;
/// Large diffs are returned to the caller without retaining them in the cache.
const CACHE_MAX_DELTAS: usize = 1_024;

/// Owned fields needed from a Git delta after its diff is released.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TreeDelta {
    /// Change classification, using libgit2's default diff semantics.
    pub status: Delta,
    /// Previous path relative to the snapshot root.
    pub old_path: Option<String>,
    /// Resulting path relative to the snapshot root.
    pub new_path: Option<String>,
    /// Resulting object identity, or zero for a deletion.
    pub new_id: Oid,
}

/// One reusable comparison between immutable trees.
struct CachedTreeDiff {
    /// Previous tree, absent for an initial snapshot.
    old: Option<Oid>,
    /// Resulting tree.
    new: Oid,
    /// Owned file changes relative to the snapshot root.
    deltas: Arc<[TreeDelta]>,
}

/// A bounded, scan-local cache keyed by immutable tree identities.
#[derive(Default)]
pub(super) struct TreeDiffCache {
    /// Recently used comparisons; no repository state survives the current scan.
    entries: VecDeque<CachedTreeDiff>,
    /// Native subtree diffs attempted, excluding unchanged directories and cache hits.
    pub diffs_attempted: u64,
    /// Comparisons reused when history repeats an earlier tree transition.
    pub cache_hits: u64,
}

impl TreeDiffCache {
    /// Compares snapshots while preserving per-commit changes, including reverts.
    pub fn diff(
        &mut self,
        repo: &Repository,
        old: Option<&Tree<'_>>,
        new: &Tree<'_>,
    ) -> Result<Arc<[TreeDelta]>> {
        let key = (old.map(Tree::id), new.id());
        if let Some(position) = self
            .entries
            .iter()
            .position(|cached| (cached.old, cached.new) == key)
            && let Some(entry) = self.entries.remove(position)
        {
            let deltas = Arc::clone(&entry.deltas);
            self.entries.push_back(entry);
            self.cache_hits += 1;
            return Ok(deltas);
        }
        let mut deltas = Vec::new();
        self.compare(repo, old, Some(new), "", &mut deltas)?;
        let deltas: Arc<[TreeDelta]> = deltas.into();
        if deltas.len() <= CACHE_MAX_DELTAS {
            if self.entries.len() == CACHE_ENTRIES {
                self.entries.pop_front();
            }
            self.entries.push_back(CachedTreeDiff {
                old: key.0,
                new: key.1,
                deltas: Arc::clone(&deltas),
            });
        }
        Ok(deltas)
    }

    /// Descends through changed directories and delegates file-level semantics to libgit2.
    fn compare(
        &mut self,
        repo: &Repository,
        old: Option<&Tree<'_>>,
        new: Option<&Tree<'_>>,
        prefix: &str,
        deltas: &mut Vec<TreeDelta>,
    ) -> Result<()> {
        if old.map(Tree::id) == new.map(Tree::id) {
            return Ok(());
        }
        let mut directories = BTreeSet::new();
        for entry in old
            .into_iter()
            .flat_map(Tree::iter)
            .chain(new.into_iter().flat_map(Tree::iter))
        {
            // Native diff already scans files efficiently. Inspecting them here first
            // doubles that work when a changed file sorts late in a wide directory.
            if entry.kind() != Some(ObjectType::Tree) {
                return self.native_diff(repo, old, new, prefix, deltas);
            }
            let Ok(name) = entry.name() else {
                return self.native_diff(repo, old, new, prefix, deltas);
            };
            let before = old.and_then(|tree| tree.get_name(name));
            let after = new.and_then(|tree| tree.get_name(name));
            let identity = |entry: &TreeEntry<'_>| (entry.id(), entry.filemode());
            if before.as_ref().map(identity) == after.as_ref().map(identity) {
                continue;
            }
            // File changes and file/directory replacements retain native diff semantics.
            if before
                .iter()
                .chain(after.iter())
                .any(|entry| entry.kind() != Some(ObjectType::Tree))
            {
                return self.native_diff(repo, old, new, prefix, deltas);
            }
            directories.insert(name.to_owned());
        }
        for name in directories {
            let before = old
                .and_then(|tree| tree.get_name(&name))
                .map(|entry| repo.find_tree(entry.id()))
                .transpose()?;
            let after = new
                .and_then(|tree| tree.get_name(&name))
                .map(|entry| repo.find_tree(entry.id()))
                .transpose()?;
            self.compare(
                repo,
                before.as_ref(),
                after.as_ref(),
                &format!("{prefix}{name}/"),
                deltas,
            )?;
        }
        Ok(())
    }

    /// Copies native deltas with their snapshot-relative paths restored.
    fn native_diff(
        &mut self,
        repo: &Repository,
        old: Option<&Tree<'_>>,
        new: Option<&Tree<'_>>,
        prefix: &str,
        deltas: &mut Vec<TreeDelta>,
    ) -> Result<()> {
        self.diffs_attempted += 1;
        let diff = repo.diff_tree_to_tree(old, new, None)?;
        for delta in diff.deltas() {
            let path = |path: Option<&Path>| {
                path.and_then(Path::to_str)
                    .map(|path| format!("{prefix}{path}"))
            };
            deltas.push(TreeDelta {
                status: delta.status(),
                old_path: path(delta.old_file().path()),
                new_path: path(delta.new_file().path()),
                new_id: delta.new_file().id(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::{Index, IndexEntry, IndexTime};
    use std::time::Instant;

    /// Creates a tree directly, keeping tests independent of sync and checkpoint logic.
    fn snapshot(repo: &Repository, files: &[(&str, &[u8], u32)]) -> Oid {
        let mut index = Index::new().unwrap();
        for (path, content, mode) in files {
            index
                .add(&IndexEntry {
                    ctime: IndexTime::new(0, 0),
                    mtime: IndexTime::new(0, 0),
                    dev: 0,
                    ino: 0,
                    mode: *mode,
                    uid: 0,
                    gid: 0,
                    file_size: content.len() as u32,
                    id: repo.blob(content).unwrap(),
                    flags: 0,
                    flags_extended: 0,
                    path: path.as_bytes().to_vec(),
                })
                .unwrap();
        }
        index.write_tree_to(repo).unwrap()
    }

    /// Compares full-root native deltas against pruned and cached comparisons in both directions.
    #[test]
    fn matches_native_diffs_for_directory_and_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init_bare(dir.path()).unwrap();
        let snapshots = [
            snapshot(&repo, &[]),
            snapshot(
                &repo,
                &[
                    ("example.com/a.json", b"a", 0o100644),
                    ("example.com/nested/b.json", b"b", 0o100644),
                    ("metadata/provider-metadata.json", b"{}", 0o100644),
                    ("untouched/x.json", b"x", 0o100644),
                ],
            ),
            snapshot(
                &repo,
                &[
                    ("example.com/a.json", b"changed", 0o100644),
                    ("example.com/a.json.asc", b"signature", 0o100644),
                    ("example.com/nested/b.json", b"b", 0o100644),
                    (
                        "metadata/provider-metadata.json",
                        b"{\"last_updated\":1}",
                        0o100644,
                    ),
                    ("untouched/x.json", b"x", 0o100644),
                ],
            ),
            snapshot(
                &repo,
                &[
                    ("example.com/renamed.json", b"a", 0o100644),
                    ("moved/nested/b.json", b"b", 0o100644),
                    ("literal[1]/a*.json", b"literal", 0o100644),
                ],
            ),
            snapshot(
                &repo,
                &[
                    ("example.com/a.json", b"a", 0o100755),
                    ("example.com/nested", b"target", 0o120000),
                ],
            ),
            snapshot(
                &repo,
                &[
                    ("example.com/a.json/child.json", b"child", 0o100644),
                    ("example.com/nested/b.json", b"b", 0o100644),
                    ("example.com/nested.ext", b"extension", 0o100644),
                ],
            ),
            snapshot(
                &repo,
                &[("example.com", b"file replaces directory", 0o100644)],
            ),
        ];
        let mut cache = TreeDiffCache::default();
        for old in std::iter::once(None).chain(snapshots.iter().copied().map(Some)) {
            let old = old.map(|id| repo.find_tree(id).unwrap());
            for new in &snapshots {
                let new = repo.find_tree(*new).unwrap();
                let mut expected = Vec::new();
                // Bypass pruning and caching for the reference full-root comparison.
                TreeDiffCache::default()
                    .native_diff(&repo, old.as_ref(), Some(&new), "", &mut expected)
                    .unwrap();
                expected.sort_by_cached_key(|delta| format!("{delta:?}"));
                for _ in 0..2 {
                    let mut actual = cache.diff(&repo, old.as_ref(), &new).unwrap().to_vec();
                    actual.sort_by_cached_key(|delta| format!("{delta:?}"));
                    assert_eq!(actual, expected);
                }
            }
        }
        assert!(cache.cache_hits > 0);
    }

    /// Metadata-only changes skip advisory trees and repeated comparisons reuse owned deltas.
    #[test]
    fn skips_unchanged_trees_and_reuses_comparisons() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init_bare(dir.path()).unwrap();
        let old = repo
            .find_tree(snapshot(
                &repo,
                &[
                    ("advisories/a.json", b"a", 0o100644),
                    ("metadata/provider.json", b"old", 0o100644),
                ],
            ))
            .unwrap();
        let new = repo
            .find_tree(snapshot(
                &repo,
                &[
                    ("advisories/a.json", b"a", 0o100644),
                    ("metadata/provider.json", b"new", 0o100644),
                ],
            ))
            .unwrap();
        let mut cache = TreeDiffCache::default();
        assert!(cache.diff(&repo, Some(&old), &old).unwrap().is_empty());
        assert_eq!(cache.diffs_attempted, 0);
        let delta = cache.diff(&repo, Some(&old), &new).unwrap();
        assert_eq!(delta.len(), 1);
        assert_eq!(delta[0].new_path.as_deref(), Some("metadata/provider.json"));
        assert_eq!(cache.diffs_attempted, 1);
        assert_eq!(cache.diff(&repo, Some(&old), &new).unwrap(), delta);
        assert_eq!(cache.diffs_attempted, 1);
        assert_eq!(cache.cache_hits, 1);
        let reverse = cache.diff(&repo, Some(&new), &old).unwrap();
        assert_ne!(reverse[0].new_id, delta[0].new_id);
    }

    /// Evicting old comparisons changes only performance, never their results.
    #[test]
    fn bounds_cache_and_recomputes_evicted_pairs() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init_bare(dir.path()).unwrap();
        let mut cache = TreeDiffCache::default();
        let initial = repo
            .find_tree(snapshot(&repo, &[("a.json", b"initial", 0o100644)]))
            .unwrap();
        let expected = cache.diff(&repo, None, &initial).unwrap();
        for n in 0..CACHE_ENTRIES {
            let tree = repo
                .find_tree(snapshot(
                    &repo,
                    &[("a.json", n.to_string().as_bytes(), 0o100644)],
                ))
                .unwrap();
            cache.diff(&repo, None, &tree).unwrap();
        }
        assert_eq!(cache.entries.len(), CACHE_ENTRIES);
        assert_eq!(cache.diff(&repo, None, &initial).unwrap(), expected);
        assert_eq!(cache.cache_hits, 0);
        assert_eq!(cache.entries.len(), CACHE_ENTRIES);

        let paths: Vec<_> = (0..=CACHE_MAX_DELTAS)
            .map(|n| format!("large/{n}.json"))
            .collect();
        let files: Vec<_> = paths
            .iter()
            .map(|path| (path.as_str(), b"{}".as_slice(), 0o100644))
            .collect();
        let large = repo.find_tree(snapshot(&repo, &files)).unwrap();
        let mut cache = TreeDiffCache::default();
        assert_eq!(
            cache.diff(&repo, None, &large).unwrap().len(),
            CACHE_MAX_DELTAS + 1
        );
        assert!(cache.entries.is_empty(), "large diffs must not be retained");
    }

    /// Compares uncached native and optimized diffs when the last file in a wide tree changes.
    #[test]
    #[ignore = "manual performance fixture: 10000 files with a change at the end"]
    fn benchmark_flat_tree_tail_change() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init_bare(dir.path()).unwrap();
        let paths: Vec<_> = (0..10_000).map(|n| format!("{n:05}.json")).collect();
        let mut files: Vec<_> = paths
            .iter()
            .map(|path| (path.as_str(), b"old".as_slice(), 0o100644))
            .collect();
        let old = repo.find_tree(snapshot(&repo, &files)).unwrap();
        files.last_mut().unwrap().1 = b"new";
        let new = repo.find_tree(snapshot(&repo, &files)).unwrap();
        let started = Instant::now();
        for _ in 0..100 {
            let diff = repo
                .diff_tree_to_tree(Some(&old), Some(&new), None)
                .unwrap();
            assert_eq!(diff.deltas().len(), 1);
        }
        let native = started.elapsed();
        let started = Instant::now();
        for _ in 0..100 {
            let diff = TreeDiffCache::default()
                .diff(&repo, Some(&old), &new)
                .unwrap();
            assert_eq!(diff.len(), 1);
            assert_eq!(diff[0].new_path.as_deref(), Some("09999.json"));
        }
        eprintln!(
            "100 uncached tail changes: native={native:?} optimized={:?}",
            started.elapsed()
        );
    }
}
