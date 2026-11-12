//! Deterministic ordering helpers.

use crate::model::Candidate;

/// Sorts candidates by their stable source, symbol, and kind key.
pub fn sort_candidates(candidates: &mut [Candidate]) {
    candidates.sort_by(|left, right| {
        (&left.source_path, &left.symbol_name, &left.kind).cmp(&(
            &right.source_path,
            &right.symbol_name,
            &right.kind,
        ))
    });
}

#[cfg(test)]
mod tests {
    use crate::model::Candidate;

    use super::sort_candidates;

    #[test]
    fn orders_by_the_stable_key_components() {
        let mut candidates = vec![candidate("z.ts", "A"), candidate("a.ts", "Z")];
        sort_candidates(&mut candidates);
        assert_eq!(candidates[0].source_path, "a.ts");
    }

    fn candidate(source_path: &str, symbol_name: &str) -> Candidate {
        Candidate {
            id: String::new(),
            declaration_key: String::new(),
            stable_key: format!("{source_path}#{symbol_name}#typeAlias"),
            declaration_stable_key: String::new(),
            symbol_name: symbol_name.into(),
            kind: "typeAlias".into(),
            source_path: source_path.into(),
            destination_path: "src/types/shared.ts".into(),
            referrer_paths: vec![],
            referrer_count: 0,
            counted_referrer_count: None,
            local_dependency_names: vec![],
            local_dependency_declaration_keys: vec![],
            referrer_strategies: vec![],
            readonly_referrers: vec![],
            analysis_limitations: vec![],
            destination_index_path: None,
        }
    }
}
