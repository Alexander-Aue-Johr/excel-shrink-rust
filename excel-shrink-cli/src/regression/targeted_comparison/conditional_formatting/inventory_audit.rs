//! Opt-in offline eligibility audit; counts syntax fallbacks, never claims unchanged inputs.
use super::*;
use anyhow::ensure;
use serde_json::{json, Value};
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read},
    path::PathBuf,
};

#[test]
#[ignore = "Requires CF_INVENTORY, CF_AUDIT_INPUT and a new CF_COST_OUTPUT; no Excel"]
fn rank_fallbacks_with_source_visual_extents() -> Result<()> {
    let root = PathBuf::from(std::env::var("CF_INVENTORY")?);
    let input: Value = serde_json::from_slice(&fs::read(std::env::var("CF_AUDIT_INPUT")?)?)?;
    let output = PathBuf::from(std::env::var("CF_COST_OUTPUT")?);
    ensure!(!output.exists(), "Use a new cost output");
    let mut groups = BTreeMap::<u64, Vec<&Value>>::new();
    for block in input["fallbacks"].as_array().context("Missing fallbacks")? {
        groups
            .entry(block["source_id"].as_u64().unwrap())
            .or_default()
            .push(block);
    }
    let mut costs = Vec::new();
    let mut outside_visual_plan = Vec::new();
    for (id, blocks) in groups {
        let plan = crate::regression::office_plan::build_workbook_visual_plan(
            &root.join(format!("sources/{id:05}.xlsx")),
        )?;
        let mut candidates = BTreeSet::new();
        let mut planned_blocks = 0;
        for block in &blocks {
            let Some(sheet) = plan
                .sheets
                .iter()
                .find(|sheet| sheet.sheet_file == block["part"].as_str().unwrap())
            else {
                outside_visual_plan.push((*block).clone());
                continue;
            };
            planned_blocks += 1;
            let extent = Rect {
                top: 1,
                left: 1,
                bottom: sheet.bottom_row + 1,
                right: sheet.base_right_column + 1,
            };
            let areas =
                ranges::areas(block["sqref"].as_str().unwrap()).unwrap_or_else(|| vec![extent]);
            fallback(&sheet.sheet, &areas, extent, &mut candidates);
        }
        let mut by_sheet = BTreeMap::<String, usize>::new();
        for candidate in &candidates {
            *by_sheet.entry(candidate.sheet.clone()).or_default() += 1;
        }
        costs.push(json!({"source_id":id,"workbook":blocks[0]["workbook"],
            "blocks":blocks.len(),"planned_blocks":planned_blocks,"candidate_rectangles":candidates.len(),"by_sheet":by_sheet}));
    }
    costs.sort_by_key(|row| std::cmp::Reverse(row["candidate_rectangles"].as_u64().unwrap()));
    fs::write(
        output,
        serde_json::to_vec_pretty(&json!({
            "scope":"Static fallback rectangles clipped to original source visual extents; exact duplicate rectangles removed. Not measured capture time or full pairwise pipeline job count.",
            "workbooks":costs,
            "outside_source_visual_plan":outside_visual_plan,
        }))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "Requires CF_INVENTORY and a new CF_AUDIT_OUTPUT; scans retained CF sheets without Excel"]
fn audit_inventory_with_current_selector() -> Result<()> {
    let root = PathBuf::from(std::env::var("CF_INVENTORY")?);
    let output = PathBuf::from(std::env::var("CF_AUDIT_OUTPUT")?);
    ensure!(!output.exists(), "Use a new audit output");
    let mut names = BTreeMap::new();
    let mut defined_names = BTreeMap::new();
    let mut fallback = Vec::new();
    let mut supported: BTreeMap<&str, usize> = BTreeMap::new();
    let mut errors = Vec::new();
    let (mut total, mut scanned) = (0, 0);
    for line in BufReader::new(File::open(root.join("sheets.jsonl"))?).lines() {
        let sheet: Value = serde_json::from_str(&line?)?;
        if sheet["blocks"]
            .as_array()
            .is_none_or(|blocks| blocks.is_empty())
        {
            continue;
        }
        let id = sheet["source_id"].as_u64().context("Source ID missing")?;
        let part = sheet["part"].as_str().context("Sheet part missing")?;
        let mut archive =
            zip::ZipArchive::new(File::open(root.join(format!("sources/{id:05}.xlsx")))?)?;
        if !names.contains_key(&id) {
            let mut bytes = Vec::new();
            archive
                .by_name("xl/workbook.xml")?
                .read_to_end(&mut bytes)?;
            names.insert(id, xml::sheet_names(&bytes)?);
            defined_names.insert(id, xml::defined_names_in_package(&mut archive, &bytes)?);
        }
        let catalogue =
            match super::super::xml_changes::conditional_catalogue_for_audit(&mut archive, part) {
                Ok(catalogue) => catalogue,
                Err(error) => {
                    errors.push(json!({"source_id":id,"part":part,"error":format!("{error:#}")}));
                    continue;
                }
            };
        let snapshot = Snapshot {
            sheets: BTreeMap::new(),
            context: String::new(),
            cf_context: String::new(),
            sheet_names: names[&id].clone(),
            defined_names: defined_names[&id].clone(),
        };
        for (index, block) in catalogue.blocks.iter().enumerate() {
            total += 1;
            let mode = if !catalogue.namespace_supported[index] {
                Proof::Fallback("namespace-or-extension")
            } else if ranges::areas(&block.sqref).is_none() {
                Proof::Fallback("application-range-not-resolved")
            } else {
                proof(block, &snapshot, &snapshot)
            };
            let reason = match mode {
                Proof::Fallback(reason) => reason,
                mode => {
                    let kind = match mode {
                        Proof::LocalInputs => "literal",
                        Proof::WholeAreaInputs => "whole-area",
                        Proof::ReferencedInputs(_) => "references",
                        Proof::CombinedInputs(_) => "combined",
                        Proof::MissingDummyTargets => "dummy",
                        Proof::InputIndependent(_) => "input-independent",
                        Proof::Fallback(_) => unreachable!(),
                    };
                    *supported.entry(kind).or_default() += 1;
                    continue;
                }
            };
            let family = block
                .rules
                .iter()
                .filter_map(|rule| rule.attributes.get("type"))
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join("+");
            fallback.push(json!({"source_id":id,"workbook":sheet["workbook"],"part":part,
                "block":index,"reason":reason,"family":family,"sqref":block.sqref,
                "tiles":block.tile_proxy_upper_bound,"formulas":block.rules.iter().flat_map(|rule| &rule.formulas).collect::<Vec<_>>()}));
        }
        scanned += 1;
        if scanned % 100 == 0 {
            eprintln!("Audited {scanned} CF sheets; {total} blocks");
        }
    }
    fallback.sort_by_key(|block| std::cmp::Reverse(block["tiles"].as_u64().unwrap_or(0)));
    let mut reasons = BTreeMap::<String, usize>::new();
    for block in &fallback {
        *reasons
            .entry(block["reason"].as_str().unwrap().into())
            .or_default() += 1;
    }
    fs::write(
        output,
        serde_json::to_vec_pretty(&json!({
            "scope":"Static rule eligibility using actual namespace scopes; input/context changes may require further captures",
            "tile_grid":"256 rows by 32 columns, unclipped summed upper-bound proxy; not actual capture jobs",
            "total":total,"scanned_sheets":scanned,"supported":supported,"remaining":fallback.len(),
            "reasons":reasons,"fallbacks":fallback,"errors":errors,
        }))?,
    )?;
    ensure!(errors.is_empty(), "Audit had unreadable sheets; see output");
    Ok(())
}
