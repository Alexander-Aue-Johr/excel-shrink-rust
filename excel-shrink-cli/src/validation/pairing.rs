//! Deterministic pairing without silently choosing one of several equal filenames.

use std::{collections::{BTreeMap, BTreeSet}, path::{Path, PathBuf}};

#[derive(Debug)]
pub(super) struct WorkbookPair {
    pub name: String,
    pub original: PathBuf,
    pub candidate: Option<PathBuf>,
    pub candidate_name: Option<String>,
    pub problem: Option<String>,
}

pub(super) struct Pairing {
    pub pairs: Vec<WorkbookPair>,
    pub unmatched_candidates: Vec<String>,
}

/// Prefer relative names, then unique basenames, before allowing changed extensions.
/// Both input sets participate in uniqueness; traversal order can never decide a pair.
pub(super) fn pair_files(
    originals: &BTreeMap<String, PathBuf>,
    candidates: &BTreeMap<String, PathBuf>,
) -> Pairing {
    let mut left: BTreeSet<_> = originals.keys().cloned().collect();
    let mut right: BTreeSet<_> = candidates.keys().cloned().collect();
    let mut matches = BTreeMap::new();
    let mut ambiguous = BTreeSet::new();
    for (basename, stem) in [(false, false), (true, false), (false, true), (true, true)] {
        let left_index = index(&left, basename, stem);
        let right_index = index(&right, basename, stem);
        for (key, original_names) in left_index {
            let Some(candidate_names) = right_index.get(&key) else { continue };
            if original_names.len() == 1 && candidate_names.len() == 1 {
                let original = &original_names[0];
                let candidate = &candidate_names[0];
                matches.insert(original.clone(), candidate.clone());
                left.remove(original);
                right.remove(candidate);
            } else {
                ambiguous.extend(original_names);
            }
        }
    }
    let pairs = originals.iter().map(|(name, path)| {
        let candidate_name = matches.get(name).cloned();
        let candidate = candidate_name.as_ref().map(|name| candidates[name].clone());
        let problem = candidate.is_none().then(|| {
            if ambiguous.contains(name) {
                "Ambiguous filename match; preserve distinct relative paths on both sides".to_owned()
            } else {
                "No matching candidate file was produced or supplied".to_owned()
            }
        });
        WorkbookPair { name: name.clone(), original: path.clone(), candidate, candidate_name, problem }
    }).collect();
    Pairing { pairs, unmatched_candidates: right.into_iter().collect() }
}

fn index(names: &BTreeSet<String>, basename: bool, stem: bool) -> BTreeMap<String, Vec<String>> {
    let mut result = BTreeMap::<String, Vec<String>>::new();
    for name in names {
        let path = Path::new(name);
        let key = if basename {
            if stem { path.file_stem() } else { path.file_name() }
                .unwrap_or_default().to_string_lossy().into_owned()
        } else if stem {
            path.with_extension("").to_string_lossy().replace('\\', "/")
        } else { name.clone() };
        result.entry(key.to_lowercase()).or_default().push(name.clone());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn files(names: &[&str]) -> BTreeMap<String, PathBuf> {
        names.iter().map(|name| ((*name).into(), PathBuf::from(name))).collect()
    }

    #[test]
    fn exact_paths_disambiguate_repeated_names_and_unique_basenames_allow_archive_roots() {
        let a = files(&["one/book.xlsx", "two/book.xlsx", "original/unique.xlsx"]);
        let b = files(&["two/book.xlsx", "one/book.xlsx", "processed/unique.xlsx"]);
        let paired = pair_files(&a, &b);
        assert!(paired.pairs.iter().all(|pair| pair.problem.is_none()));
        assert_eq!(paired.pairs[0].candidate_name.as_deref(), Some("one/book.xlsx"));
        assert!(paired.unmatched_candidates.is_empty());
    }

    #[test]
    fn ambiguous_names_never_depend_on_iteration_order() {
        let paired = pair_files(&files(&["a/book.xlsx", "b/book.xlsx"]), &files(&["book.xlsx"]));
        assert!(paired.pairs.iter().all(|pair| pair.candidate.is_none()));
        assert_eq!(paired.unmatched_candidates, ["book.xlsx"]);
    }

    #[test]
    fn conversion_pairs_stems_but_does_not_reuse_a_candidate_twice() {
        let paired = pair_files(&files(&["book.xls", "missing.xls"]), &files(&["book.xlsx"]));
        assert_eq!(paired.pairs[0].candidate_name.as_deref(), Some("book.xlsx"));
        assert!(paired.pairs[1].candidate.is_none());
    }
}
