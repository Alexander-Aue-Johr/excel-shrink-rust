//! Prioritize comparisons using the current preparation's measured XLSX sizes.
use super::*;

fn sizes(record: &Value) -> Option<(u64, u64)> {
    let profile = record["results"]
        .as_array()?
        .iter()
        .find(|p| p["profile"] == "default")?;
    let original = profile["original_bytes"].as_u64()?;
    (original > 0).then_some((original, profile["shrunk_bytes"].as_u64()?))
}

/// Compare the retained fractions exactly: a smaller retained fraction means a
/// larger relative reduction. Cross-products fit u128 for any two u64 sizes.
pub(super) fn sort(jobs: &mut [scheduler::Comparison], records: &[Value]) {
    jobs.sort_by(|left, right| {
        let order = match (
            sizes(&records[left.position]),
            sizes(&records[right.position]),
        ) {
            (Some((lo, ls)), Some((ro, rs))) => {
                (u128::from(ls) * u128::from(ro)).cmp(&(u128::from(rs) * u128::from(lo)))
            }
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        };
        order.then_with(|| left.id.cmp(&right.id))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relative_reduction_precedes_absolute_savings_and_invalid_sizes() {
        let measurements = [(1000, 1), (1000000, 10000), (100, 200), (0, 0), (100, 1)];
        let records: Vec<_> = measurements.into_iter().map(|(original, shrunk)|
            json!({"results":[{"profile":"default","original_bytes":original,"shrunk_bytes":shrunk}]})).collect();
        let mut jobs: Vec<_> = (0..records.len())
            .map(|position| scheduler::Comparison {
                index: position,
                position,
                id: position.to_string(),
            })
            .collect();
        sort(&mut jobs, &records);
        assert_eq!(
            jobs.iter().map(|j| j.position).collect::<Vec<_>>(),
            [0, 1, 4, 2, 3]
        );
    }
}
