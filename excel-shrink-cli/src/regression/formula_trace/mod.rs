//! Optional formula explanations. They describe dependencies, never change a
//! visual verdict, and never equate a volatile path with exclusive causation.
mod syntax;
mod workbook;

use anyhow::{ensure, Result};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::Path,
};
use syntax::{Address, Dependency};

const MAX_EXAMPLES: usize = 20;
const SEARCH_WORK: usize = 100_000;

pub(crate) struct Workbook(workbook::Workbook);

pub(crate) fn read(path: &Path) -> Result<Workbook> {
    Ok(Workbook(workbook::read(path)?))
}

/// Index formulas and explicit audit targets while leaving unrelated constants
/// to the full cell comparator. This only controls optional dependency evidence.
pub(crate) fn read_for_cells(path: &Path, targets: &[(String, String)]) -> Result<Workbook> {
    let selected = targets
        .iter()
        .map(|(sheet, cell)| Address::parse(sheet, cell))
        .collect::<Result<BTreeSet<_>>>()?;
    Ok(Workbook(workbook::read_selected(path, &selected)?))
}

/// Explain changed saved values in a selected capture; missing/unsupported
/// evidence remains explicit. Counts refer to cells, not changed pixels.
pub(crate) fn compare(
    left: &Workbook,
    right: &Workbook,
    sheet: &str,
    range: &str,
) -> Result<Value> {
    let bounds = bounds(sheet, range)?;
    let keys: BTreeSet<_> = left
        .0
        .keys()
        .chain(right.0.keys())
        .filter(|key| inside(key, &bounds))
        .cloned()
        .collect();
    let mut changed = 0;
    let mut traced = 0;
    let mut paths = 0;
    let mut environment_paths = 0;
    let mut examples = Vec::new();
    let mut budget = SEARCH_WORK;
    for key in keys {
        let a = left.0.get(&key);
        let b = right.0.get(&key);
        if a.and_then(|c| c.value.as_ref()).is_none() && b.and_then(|c| c.value.as_ref()).is_none()
        {
            continue;
        }
        if a.zip(b).is_some_and(|(a, b)| a.same_value(b)) {
            continue;
        }
        changed += 1;
        if budget == 0 {
            continue;
        }
        traced += 1;
        let path = find_path(&right.0, &key, &mut budget);
        let example = match path {
            Ok(Some(path)) => match nodes(&left.0, &right.0, &path) {
                Ok(nodes) => {
                    let environment = nodes.last().is_some_and(|node| {
                        node["volatile_span"].is_null() && !node["environment_span"].is_null()
                    });
                    if environment {
                        environment_paths += 1;
                    } else {
                        paths += 1;
                    }
                    json!({"status":"path_found","dependency_kind":if environment {"environment"} else {"random_time"},"path":nodes})
                }
                Err(error) => {
                    json!({"status":"not_resolved","cell":key.cell(),"reason":format!("{error:#}")})
                }
            },
            Ok(None) => {
                json!({"status":"no_supported_path","cell":key.cell(),"left_value":a.and_then(|c| c.value.clone()),"right_value":b.and_then(|c| c.value.clone())})
            }
            Err(error) => {
                json!({"status":"not_resolved","cell":key.cell(),"reason":format!("{error:#}")})
            }
        };
        if examples.len() < MAX_EXAMPLES {
            examples.push(example);
        }
    }
    Ok(
        json!({"version":1,"evidence":"saved_compared_states","sheet":sheet,"range":range,
        "changed_value_cells":changed,"examined_cells":traced,"volatile_paths":paths,
        "environment_paths":environment_paths,
        "coverage_complete":changed==traced,"examples":examples,
        "all_changed_values_have_volatile_dependency":changed>0 && changed==traced && paths==changed,
        "exclusive_cause_proven":false,"note":"A formula dependency is not proof that all visual differences have this cause."}),
    )
}

/// Older runs did not retain compared values. Show source dependency examples
/// honestly, without presenting source cached values as rebuilt capture values.
pub(crate) fn source_examples(source: &Workbook, sheet: &str, range: &str) -> Result<Value> {
    let bounds = bounds(sheet, range)?;
    let mut examples = Vec::new();
    let mut examined = 0;
    let mut budget = SEARCH_WORK;
    for (key, cell) in &source.0 {
        if !inside(key, &bounds) || cell.formula.is_none() {
            continue;
        }
        if examined >= 1000 || budget == 0 {
            break;
        }
        examined += 1;
        if let Ok(Some(path)) = find_path(&source.0, key, &mut budget) {
            if let Ok(nodes) = nodes(&source.0, &source.0, &path) {
                let environment = nodes.last().is_some_and(|node| {
                    node["volatile_span"].is_null() && !node["environment_span"].is_null()
                });
                examples.push(json!({"status":"path_found","dependency_kind":if environment {"environment"} else {"random_time"},"path":nodes}));
                examples.sort_by_key(|example| {
                    std::cmp::Reverse(example["path"].as_array().map_or(0, Vec::len))
                });
                examples.truncate(MAX_EXAMPLES);
            }
        }
    }
    Ok(
        json!({"version":1,"evidence":"prepared_source_cached_values","sheet":sheet,"range":range,
        "examined_cells":examined,
        "volatile_paths":examples.iter().filter(|example| example["dependency_kind"]=="random_time").count(),
        "environment_paths":examples.iter().filter(|example| example["dependency_kind"]=="environment").count(),"examples":examples,
        "coverage_complete":false,"exclusive_cause_proven":false,
        "note":"Compared-state values are unavailable for this older result. These are source formula dependencies and source cached values, not captured rebuilt values."}),
    )
}

fn bounds(sheet: &str, range: &str) -> Result<(Address, Address)> {
    let (start, end) = range.split_once(':').unwrap_or((range, range));
    Ok((Address::parse(sheet, start)?, Address::parse(sheet, end)?))
}

fn inside(address: &Address, (a, b): &(Address, Address)) -> bool {
    address.sheet == a.sheet
        && (a.column.min(b.column)..=a.column.max(b.column)).contains(&address.column)
        && (a.row.min(b.row)..=a.row.max(b.row)).contains(&address.row)
}

/// Search one supported path; explicit budgets and visited nodes bound cycles
/// and large range fan-out. Unknown branches cannot certify absence of volatility.
fn find_path(
    cells: &workbook::Workbook,
    start: &Address,
    work: &mut usize,
) -> Result<Option<Vec<Address>>> {
    let mut queue = VecDeque::from([start.clone()]);
    let mut parents: BTreeMap<Address, Option<Address>> = BTreeMap::from([(start.clone(), None)]);
    let mut unsupported = false;
    while let Some(key) = queue.pop_front() {
        ensure!(*work > 0, "Formula dependency search budget exceeded");
        *work -= 1;
        let Some(cell) = cells.get(&key) else {
            continue;
        };
        let Some(formula) = &cell.formula else {
            continue;
        };
        if syntax::volatile_span(formula).is_some() || syntax::environment_span(formula).is_some() {
            let mut path = vec![key.clone()];
            let mut previous = &key;
            while let Some(Some(parent)) = parents.get(previous) {
                path.push(parent.clone());
                previous = parent;
            }
            ensure!(
                path.len() <= 64,
                "Formula path is too long for the inline diagnostic"
            );
            path.reverse();
            return Ok(Some(path));
        }
        let dependencies = match syntax::references(formula, &key.sheet) {
            Ok(dependencies) => dependencies,
            Err(_) => {
                unsupported = true;
                continue;
            }
        };
        for dependency in dependencies {
            let mut add = |next: &Address| {
                if !parents.contains_key(next) {
                    parents.insert(next.clone(), Some(key.clone()));
                    queue.push_back(next.clone());
                }
            };
            match dependency {
                Dependency::Cell(next) => add(&next),
                Dependency::Range(a, b) => {
                    for next in cells.keys().filter(|next| next.sheet == a.sheet) {
                        ensure!(*work > 0, "Formula range search budget exceeded");
                        *work -= 1;
                        if (a.column.min(b.column)..=a.column.max(b.column)).contains(&next.column)
                            && (a.row.min(b.row)..=a.row.max(b.row)).contains(&next.row)
                        {
                            add(next);
                        }
                    }
                }
            }
        }
    }
    ensure!(
        !unsupported,
        "No supported path found; some dynamic references remain unresolved"
    );
    Ok(None)
}

/// Retain both saved values and expose call spans in the browser's UTF-16 coordinates.
fn nodes(
    left: &workbook::Workbook,
    right: &workbook::Workbook,
    path: &[Address],
) -> Result<Vec<Value>> {
    path.iter()
        .map(|key| {
            let before = left.get(key);
            let after = right.get(key);
            let formula = after.and_then(|cell| cell.formula.as_deref()).unwrap_or("");
            ensure!(formula.len() <= 4096, "Formula is too long for an inline explanation");
            for cell in [before, after].into_iter().flatten() {
                ensure!(
                    cell.value.as_ref().is_none_or(|value| value.len() <= 512),
                    "Value is too long for an inline explanation"
                );
            }
            let browser_span = |span: std::ops::Range<usize>| [
                formula[..span.start].encode_utf16().count(),
                formula[..span.end].encode_utf16().count(),
            ];
            Ok(json!({
                "sheet": key.sheet,
                "cell": key.cell(),
                "formula": formula,
                "left_formula": before.and_then(|cell| cell.formula.as_deref()),
                "left_value": before.and_then(|cell| cell.value.as_deref()),
                "right_value": after.and_then(|cell| cell.value.as_deref()),
                "shared_anchor": after.and_then(|cell| cell.shared_anchor.as_deref()),
                "volatile_span": syntax::volatile_span(formula).map(browser_span),
                "environment_span": syntax::environment_span(formula).map(browser_span),
            }))
        })
        .collect()
}

#[cfg(test)]
mod tests;
