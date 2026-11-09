//! Deterministic size partitions are computed before resume or priority filters.
use super::*;
use serde::Serialize;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub(super) enum ProcessingOrder {
    #[default]
    Priority,
    SizeAsc,
    SizeDesc,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(super) struct SizePart {
    pub part: u32,
    pub parts: u32,
}

impl FromStr for SizePart {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        let (part, parts) = value
            .split_once('/')
            .ok_or("expected K/N, for example 1/4")?;
        let part = part
            .parse::<u32>()
            .map_err(|_| "K must be a positive integer")?;
        let parts = parts
            .parse::<u32>()
            .map_err(|_| "N must be a positive integer")?;
        if part == 0 || parts == 0 || part > parts {
            return Err("expected 1 <= K <= N; 1/N contains the smallest files".into());
        }
        Ok(Self { part, parts })
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct CatalogMember {
    pub archive_index: usize,
    pub member_name: String,
    // A corrupt local header may have no readable size. Keep it in the catalog
    // and place it after known sizes, so one worker can still report its error.
    pub bytes: Option<u64>,
    pub crc32: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct RankedMember {
    #[serde(flatten)]
    pub member: CatalogMember,
    pub rank: usize,
    pub selected: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct Plan {
    pub size_part: Option<SizePart>,
    pub total_workbooks: usize,
    pub selected_workbooks: usize,
    pub first_selected_rank: Option<usize>,
    pub last_selected_rank: Option<usize>,
    pub smallest_selected_bytes: Option<u64>,
    pub largest_selected_bytes: Option<u64>,
    pub catalog_sha256: String,
    pub members: Vec<RankedMember>,
    #[serde(skip)]
    by_index: std::collections::HashMap<usize, usize>,
}

impl Plan {
    pub fn new(mut members: Vec<CatalogMember>, size_part: Option<SizePart>) -> Result<Self> {
        members.sort_by(|a, b| {
            (a.bytes.is_none(), a.bytes, &a.member_name, a.archive_index).cmp(&(
                b.bytes.is_none(),
                b.bytes,
                &b.member_name,
                b.archive_index,
            ))
        });
        let catalog_sha256 = artifact_cache::sha256(&serde_json::to_vec(&members)?);
        let count = members.len();
        let (start, end) = size_part.map_or((0, count), |p| {
            let boundary = |part: u32| ((count as u128 * part as u128) / p.parts as u128) as usize;
            (boundary(p.part - 1), boundary(p.part))
        });
        let selected = &members[start..end];
        Ok(Self {
            size_part,
            total_workbooks: count,
            selected_workbooks: end - start,
            first_selected_rank: (start < end).then_some(start + 1),
            last_selected_rank: (start < end).then_some(end),
            smallest_selected_bytes: selected.first().and_then(|m| m.bytes),
            largest_selected_bytes: selected.last().and_then(|m| m.bytes),
            catalog_sha256,
            by_index: members
                .iter()
                .enumerate()
                .map(|(rank, m)| (m.archive_index, rank))
                .collect(),
            members: members
                .into_iter()
                .enumerate()
                .map(|(i, member)| RankedMember {
                    member,
                    rank: i + 1,
                    selected: (start..end).contains(&i),
                })
                .collect(),
        })
    }

    pub fn contains(&self, archive_index: usize) -> bool {
        self.by_index
            .get(&archive_index)
            .is_some_and(|&i| self.members[i].selected)
    }

    fn order_members(&self, members: &mut [(usize, String)], order: ProcessingOrder) {
        if order == ProcessingOrder::Priority {
            return;
        }
        members.sort_by(|(left, _), (right, _)| {
            let left = &self.members[self.by_index[left]].member;
            let right = &self.members[self.by_index[right]].member;
            left.bytes
                .is_none()
                .cmp(&right.bytes.is_none())
                .then_with(|| match order {
                    ProcessingOrder::SizeDesc => right.bytes.cmp(&left.bytes),
                    _ => left.bytes.cmp(&right.bytes),
                })
                .then_with(|| left.member_name.cmp(&right.member_name))
                .then_with(|| left.archive_index.cmp(&right.archive_index))
        });
    }

    pub fn provenance(
        &self,
        archive_index: usize,
        corpus_id: &str,
        source_fingerprint: &str,
        manifest: &str,
        order: ProcessingOrder,
    ) -> Value {
        let member = &self.members[self.by_index[&archive_index]];
        json!({
            "version":1, "manifest_sha256":manifest, "corpus_id":corpus_id,
            "source_fingerprint":source_fingerprint, "catalog_sha256":self.catalog_sha256,
            "size_part":self.size_part, "total_workbooks":self.total_workbooks,
            "first_selected_rank":self.first_selected_rank, "last_selected_rank":self.last_selected_rank,
            "archive_index":archive_index, "rank":member.rank, "bytes":member.member.bytes,
            "processing_order":order
        })
    }
}

pub(super) fn ordered_members(source: &Source, order: ProcessingOrder) -> Vec<(usize, String)> {
    // Partition membership always comes from the original ascending catalog.
    let mut members: Vec<_> = source
        .members
        .iter()
        .filter(|(index, _)| source.selection.contains(*index))
        .cloned()
        .collect();
    source.selection.order_members(&mut members, order);
    members
}

/// Validate an explicit subset against the original catalog without changing workbook identities.
pub(super) fn read_workbook_list(
    path: Option<&Path>,
    sources: &[Source],
) -> Result<Option<std::collections::BTreeMap<String, usize>>> {
    let Some(path) = path else { return Ok(None) };
    let identities: Vec<String> = serde_json::from_slice(&fs::read(path)?)?;
    let available: HashSet<_> = sources
        .iter()
        .flat_map(|source| {
            source
                .members
                .iter()
                .filter(|(index, _)| source.selection.contains(*index))
                .map(|(index, name)| identity(source, *index, name))
        })
        .collect();
    ensure!(!identities.is_empty(), "Workbook list is empty");
    let mut ranks = std::collections::BTreeMap::new();
    for (rank, id) in identities.into_iter().enumerate() {
        ensure!(
            available.contains(&id),
            "Workbook list identity is outside the selected corpus/partition: {id}"
        );
        ensure!(
            ranks.insert(id.clone(), rank).is_none(),
            "Duplicate workbook identity: {id}"
        );
    }
    Ok(Some(ranks))
}

/// Restrict only pending work; completed records and the original archive remain untouched.
pub(super) fn select_workbooks(
    source: &Source,
    pending: &mut Vec<(usize, String)>,
    ranks: &std::collections::BTreeMap<String, usize>,
) -> bool {
    let before = pending.len();
    pending.retain(|(index, name)| ranks.contains_key(&identity(source, *index, name)));
    pending.sort_by_key(|(index, name)| ranks[&identity(source, *index, name)]);
    pending.len() != before
}

pub(super) fn pending_members(
    source: &Source,
    records: &[Value],
    order: ProcessingOrder,
    retry_skipped: bool,
) -> Vec<(usize, String)> {
    let mut members = ordered_members(source, order);
    if order == ProcessingOrder::Priority {
        members
            .sort_by_key(|(index, name)| member_priority(records, &identity(source, *index, name)));
    }
    members.retain(|(index, name)| {
        let id = identity(source, *index, name);
        records
            .iter()
            .find(|record| record["identity"] == id)
            .is_none_or(|record| {
                record["complete"] != true || (retry_skipped && record["status"] == "skipped")
            })
    });
    members
}

pub(super) fn active_corpora(corpora: &[Corpus], names: &[String]) -> Result<Vec<bool>> {
    for name in names {
        ensure!(
            corpora.iter().any(|c| c.name.eq_ignore_ascii_case(name)),
            "unknown --run-corpus {name:?}"
        );
    }
    Ok(corpora
        .iter()
        .map(|c| names.is_empty() || names.iter().any(|n| c.name.eq_ignore_ascii_case(n)))
        .collect())
}

pub(super) fn restricted(corpora: &[Value]) -> bool {
    corpora
        .iter()
        .any(|c| c["selected"] == false || c["size_part"]["parts"].as_u64().is_some_and(|n| n > 1))
}

pub(super) fn complete(corpora: &[Value], records: &[Value]) -> bool {
    !restricted(corpora)
        && !corpora.is_empty()
        && corpora.iter().all(|c| {
            let Some(total) = c["total_workbooks"].as_u64() else {
                return false;
            };
            let completed: HashSet<_> = records
                .iter()
                .filter(|r| {
                    r["corpus_id"] == c["id"]
                        && r["complete"] == true
                        && r["status"] != "archive error"
                })
                .filter_map(|r| r["identity"].as_str())
                .collect();
            completed.len() as u64 == total
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workbook_subset_preserves_catalog_identity_and_cannot_reintroduce_completed_work(
    ) -> Result<()> {
        let root = super::super::tests::test_directory("workbook-subset")?;
        let path = root.join("selection.json");
        let source = Source {
            id: "example".into(),
            name: "Example".into(),
            path: PathBuf::from("unused.zip"),
            members: vec![
                (0, "a.xlsx".into()),
                (1, "b.xlsx".into()),
                (2, "c.xlsx".into()),
            ],
            fingerprint: "catalog".into(),
            member_sha256: Default::default(),
            selection: Plan::new(
                (0..3)
                    .map(|i| CatalogMember {
                        archive_index: i,
                        member_name: format!("{}.xlsx", ['a', 'b', 'c'][i]),
                        bytes: Some(10),
                        crc32: Some(0),
                    })
                    .collect(),
                None,
            )?,
        };
        fs::write(
            &path,
            serde_json::to_vec(&["example:2:c.xlsx", "example:0:a.xlsx", "example:1:b.xlsx"])?,
        )?;
        let sources = [source];
        let ranks = read_workbook_list(Some(&path), &sources)?.unwrap();
        // The resume filter already removed completed a.xlsx. Listing it must not add it back.
        let mut pending = vec![(1, "b.xlsx".into()), (2, "c.xlsx".into())];
        select_workbooks(&sources[0], &mut pending, &ranks);
        assert_eq!(pending, [(2, "c.xlsx".into()), (1, "b.xlsx".into())]);
        fs::write(&path, b"[\"example:2:c.xlsx\",\"example:2:c.xlsx\"]")?;
        assert!(read_workbook_list(Some(&path), &sources).is_err());
        fs::write(&path, b"[\"other:2:c.xlsx\"]")?;
        assert!(read_workbook_list(Some(&path), &sources).is_err());
        fs::write(&path, b"[]")?;
        assert!(read_workbook_list(Some(&path), &sources).is_err());
        fs::remove_dir_all(root)?;
        Ok(())
    }



    fn catalog() -> Vec<CatalogMember> {
        (0..19)
            .rev()
            .map(|i| CatalogMember {
                archive_index: i,
                member_name: format!("file-{:02}.xlsx", i % 5),
                bytes: Some((i % 3) as u64),
                crc32: Some(i as u32),
            })
            .collect()
    }

    #[test]
    fn size_order_uses_stable_name_index_ties_and_keeps_unknown_sizes_last() -> Result<()> {
        let make = |index, name: &str, bytes| CatalogMember {
            archive_index: index,
            member_name: name.into(),
            bytes,
            crc32: Some(0),
        };
        let catalog = vec![
            make(4, "unknown.xlsx", None),
            make(3, "a.xlsx", Some(20)),
            make(2, "b.xls", Some(10)),
            make(1, "a.xlsx", Some(10)),
            make(0, "a.xlsx", Some(10)),
        ];
        let plan = Plan::new(catalog.clone(), None)?;
        for (order, expected) in [
            (ProcessingOrder::SizeAsc, vec![0, 1, 2, 3, 4]),
            (ProcessingOrder::SizeDesc, vec![3, 0, 1, 2, 4]),
        ] {
            for reverse in [false, true] {
                let mut queue: Vec<_> = catalog
                    .iter()
                    .map(|m| (m.archive_index, m.member_name.clone()))
                    .collect();
                if reverse {
                    queue.reverse();
                }
                plan.order_members(&mut queue, order);
                assert_eq!(
                    queue.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
                    expected
                );
            }
        }
        Ok(())
    }

    #[test]
    fn default_retry_keeps_successes_complete_and_skips_outside_the_selected_partition() -> Result<()> {
        let args = StudyArgs::try_parse_from(["study", "--size-part", "3/4"])?;
        let catalog = (0..16).map(|index| CatalogMember {
            archive_index: index,
            member_name: format!("file-{index}.xlsx"),
            bytes: Some(index as u64 + 1),
            crc32: Some(index as u32),
        }).collect();
        let selection = Plan::new(catalog, args.size_part)?;
        let source = Source {
            id: "example".into(), name: "Example".into(), path: PathBuf::from("unused.zip"),
            members: (0..16).map(|index| (index, format!("file-{index}.xlsx"))).collect(),
            fingerprint: "catalog".into(), selection, member_sha256: Default::default(),
        };
        let records = vec![
            json!({"identity":"example:4:file-4.xlsx","complete":true,"status":"skipped"}),
            json!({"identity":"example:8:file-8.xlsx","complete":true,"status":"skipped","prepared":true,"error":"old failure"}),
            json!({"identity":"example:9:file-9.xlsx","complete":true,"status":"unchanged","prepared":true}),
            json!({"identity":"example:10:file-10.xlsx","complete":true,"status":"differences"}),
            json!({"identity":"example:11:file-11.xlsx","complete":true,"status":"skipped","prepared":false}),
        ];
        let before = records.clone();
        let selected = pending_members(&source, &records, ProcessingOrder::SizeAsc, args.should_retry_skipped());
        assert_eq!(selected.iter().map(|(index, _)| *index).collect::<Vec<_>>(), [8, 11]);
        assert_eq!(records, before, "Selecting retries must retain preparation and failure evidence.");
        let disabled = StudyArgs::try_parse_from(["study", "--size-part", "3/4", "--no-retry-skipped"])?;
        assert!(pending_members(&source, &records, ProcessingOrder::SizeAsc, disabled.should_retry_skipped()).is_empty());
        Ok(())
    }

    #[test]
    fn pending_size_order_keeps_quartiles_and_resume_rules() -> Result<()> {
        let catalog: Vec<_> = [80, 10, 70, 20, 60, 30, 50, 40]
            .into_iter()
            .enumerate()
            .map(|(index, bytes)| CatalogMember {
                archive_index: index,
                member_name: format!("file-{index}.xlsx"),
                bytes: Some(bytes),
                crc32: Some(index as u32),
            })
            .collect();
        let source = Source {
            id: "example".into(),
            name: "Example".into(),
            path: PathBuf::from("unused.zip"),
            members: catalog
                .iter()
                .map(|member| (member.archive_index, member.member_name.clone()))
                .collect(),
            fingerprint: "catalog".into(),
            member_sha256: Default::default(),
            selection: Plan::new(catalog, Some("2/2".parse().unwrap()))?,
        };
        let before = serde_json::to_value(&source.selection)?;
        let records = vec![
            json!({"identity":"example:0:file-0.xlsx","complete":true,"status":"unchanged"}),
            json!({"identity":"example:2:file-2.xlsx","complete":true,"status":"skipped"}),
            json!({"identity":"example:6:file-6.xlsx","complete":false,"previous_difference":true}),
        ];
        let indices = |order, retry| {
            pending_members(&source, &records, order, retry)
                .into_iter()
                .map(|(index, _)| index)
                .collect::<Vec<_>>()
        };
        assert_eq!(indices(ProcessingOrder::Priority, false), [6, 4]);
        assert_eq!(indices(ProcessingOrder::SizeAsc, false), [6, 4]);
        assert_eq!(indices(ProcessingOrder::SizeDesc, false), [4, 6]);
        assert_eq!(indices(ProcessingOrder::SizeDesc, true), [2, 4, 6]);
        assert_eq!(serde_json::to_value(&source.selection)?, before);
        assert_eq!(source.selection.first_selected_rank, Some(5));
        assert_eq!(source.selection.last_selected_rank, Some(8));
        let provenance = source.selection.provenance(
            4,
            "example",
            "catalog",
            "manifest",
            ProcessingOrder::SizeDesc,
        );
        assert_eq!(provenance["processing_order"], "size-desc");
        assert_eq!(provenance["rank"], 6);
        Ok(())
    }

    #[test]
    fn quartiles_are_disjoint_complete_and_deterministic_with_equal_sizes() -> Result<()> {
        let mut all = HashSet::new();
        let mut counts = Vec::new();
        for part in 1..=4 {
            let plan = Plan::new(catalog(), Some(SizePart { part, parts: 4 }))?;
            counts.push(plan.selected_workbooks);
            let mut reversed = catalog();
            reversed.reverse();
            assert_eq!(
                serde_json::to_value(&plan)?,
                serde_json::to_value(Plan::new(reversed, plan.size_part)?)?
            );
            for member in plan.members.iter().filter(|m| m.selected) {
                assert!(all.insert(member.member.archive_index));
            }
        }
        assert_eq!(all.len(), 19);
        assert_eq!(counts, [4, 5, 5, 5]);
        Ok(())
    }

    #[test]
    fn resuming_only_removes_completed_members_from_the_original_partition() -> Result<()> {
        let plan = Plan::new(catalog(), Some("2/4".parse().unwrap()))?;
        let selected: Vec<_> = plan
            .members
            .iter()
            .filter(|m| m.selected)
            .map(|m| m.member.archive_index)
            .collect();
        let complete: HashSet<_> = selected[..2].iter().copied().collect();
        let pending: Vec<_> = catalog()
            .into_iter()
            .filter(|m| plan.contains(m.archive_index) && !complete.contains(&m.archive_index))
            .map(|m| m.archive_index)
            .collect();
        assert_eq!(pending.len(), selected.len() - 2);
        assert!(pending.iter().all(|index| selected.contains(index)));
        assert_eq!(plan.first_selected_rank, Some(5));
        assert_eq!(plan.last_selected_rank, Some(9));
        Ok(())
    }

    #[test]
    fn corpus_filter_keeps_all_corpora_in_the_report_without_claiming_completion() -> Result<()> {
        let corpora: Vec<_> = ["Data.gov", "Destatis", "Enron"]
            .into_iter()
            .map(|name| Corpus {
                name: name.into(),
                url: name.into(),
                id: None,
                total_workbooks: None,
                profile: None,
            })
            .collect();
        let active = active_corpora(&corpora, &["enron".into()])?;
        assert_eq!(active, [false, false, true]);
        let info: Vec<_> = corpora.iter().zip(active).map(|(c, active)| json!({"id":c.name,"name":c.name,"selected":active,"total_workbooks":1})).collect();
        let records = vec![
            json!({"corpus_id":"Enron","identity":"Enron:0:file.xlsx","complete":true,"status":"unchanged"}),
        ];
        assert_eq!(info.len(), 3);
        assert!(!complete(&info, &records));
        assert!(active_corpora(&corpora, &["typo".into()]).is_err());
        Ok(())
    }

    #[test]
    fn full_completion_requires_every_catalog_member() {
        let info = vec![json!({"id":"example","total_workbooks":2})];
        let mut records = vec![
            json!({"corpus_id":"example","identity":"example:0:a.xlsx","complete":true,"status":"unchanged"}),
        ];
        assert!(!complete(&info, &records));
        records.push(json!({"corpus_id":"example","identity":"example:1:b.xlsx","complete":true,"status":"differences"}));
        assert!(complete(&info, &records));
        let mut partition = info.clone();
        partition[0]["size_part"] = json!({"part":1,"parts":4});
        assert!(!complete(&partition, &records));
    }

    #[test]
    fn validates_partition_notation_and_handles_more_workers_than_files() -> Result<()> {
        for invalid in ["0/4", "5/4", "1/0", "1", "1/4/2", "a/4", "-1/4"] {
            assert!(invalid.parse::<SizePart>().is_err());
        }
        let plan = Plan::new(catalog()[..1].to_vec(), Some("1/4".parse().unwrap()))?;
        assert_eq!(plan.selected_workbooks, 0);
        assert_eq!(plan.first_selected_rank, None);
        Ok(())
    }
    #[test]
    fn selection_only_uses_stable_ids_leaves_results_untouched_and_does_not_load_inactive_urls()
    -> Result<()> {
        use std::io::Write;
        let output = super::super::tests::test_directory("selection-only")?;
        let archive = output.join("enron.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive)?);
        for i in 0..8 {
            zip.start_file(format!("file-{i}.xlsx"), zip::write::FileOptions::default())?;
            zip.write_all(&vec![0; i / 2 + 1])?;
        }
        zip.finish()?;
        let ids = ["a".repeat(64), "b".repeat(64), "c".repeat(64)];
        let request = output.join("request.json");
        fs::write(
            &request,
            serde_json::to_vec(
                &json!({"access_url":"http://unreachable.invalid/access", "corpora":[
                    {"id":ids[0],"name":"Data.gov","url":"http://unreachable.invalid/data.zip","total_workbooks":20},
                    {"id":ids[1],"name":"Destatis","url":"http://unreachable.invalid/destatis.zip","total_workbooks":30},
                    {"id":ids[2],"name":"Enron","url":archive,"total_workbooks":8}
                ]}),
            )?,
        )?;
        let existing = json!({"identity":format!("{}:0:file-0.xlsx",ids[2]),"corpus_id":ids[2],"corpus_name":"Enron",
            "name":"file-0.xlsx","complete":true,"status":"unchanged","results":[{"marker":"retained"}]});
        let record = record_path(&output, existing["identity"].as_str().unwrap());
        write_record(&record, &existing)?;
        let before = fs::read(&record)?;
        let args = StudyArgs::try_parse_from([
            "study",
            "--request-file",
            request.to_str().unwrap(),
            "--output-root",
            output.to_str().unwrap(),
            "--run-corpus",
            "Enron",
            "--size-part",
            "2/4",
            "--order",
            "size-desc",
            "--selection-only",
        ])?;
        run(args, LogLevel::Warn)?;
        assert_eq!(fs::read(&record)?, before);
        let report: Value = serde_json::from_slice(&fs::read(output.join("diagnosis.json"))?)?;
        assert_eq!(report["corpora"].as_array().unwrap().len(), 3);
        assert_eq!(report["total_workbooks"], 58);
        assert_eq!(report["complete"], false);
        assert_eq!(report["limited"], true);
        let manifest: Value =
            serde_json::from_slice(&fs::read(output.join("selection-manifest.json"))?)?;
        assert_eq!(manifest["processing_order"], "size-desc");
        assert_eq!(manifest["sources"].as_array().unwrap().len(), 1);
        assert_eq!(manifest["sources"][0]["corpus_id"], ids[2]);
        assert_eq!(manifest["sources"][0]["selection"]["selected_workbooks"], 2);
        assert_eq!(
            manifest["sources"][0]["selection"]["first_selected_rank"],
            3
        );
        assert_eq!(manifest["sources"][0]["selection"]["last_selected_rank"], 4);
        // The full catalog, not only the pending work, is retained for review.
        assert_eq!(
            manifest["sources"][0]["selection"]["members"]
                .as_array()
                .unwrap()
                .len(),
            8
        );
        fs::remove_dir_all(output)?;
        Ok(())
    }
    #[test]
    fn ranking_uses_uncompressed_bytes_then_name_then_archive_index() -> Result<()> {
        let make = |index, name: &str, bytes| CatalogMember {
            archive_index: index,
            member_name: name.into(),
            bytes,
            crc32: Some(0),
        };
        let plan = Plan::new(
            vec![
                make(4, "unknown.xlsx", None),
                make(3, "a.xlsx", Some(20)),
                make(2, "b.xls", Some(10)),
                make(1, "a.xlsx", Some(10)),
                make(0, "a.xlsx", Some(10)),
            ],
            Some("2/2".parse().unwrap()),
        )?;
        let order: Vec<_> = plan
            .members
            .iter()
            .map(|m| m.member.archive_index)
            .collect();
        assert_eq!(order, [0, 1, 2, 3, 4]);
        assert_eq!(plan.first_selected_rank, Some(3));
        assert_eq!(plan.smallest_selected_bytes, Some(10));
        assert!(plan.contains(4));
        assert_eq!(
            plan.provenance(
                3,
                "source",
                "fingerprint",
                "manifest",
                ProcessingOrder::Priority
            )["rank"],
            4
        );
        Ok(())
    }
}
