//! Judge-reported raw observations, not an independent semantic truth oracle.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Support {
    Supported,
    Unsupported,
    Undetermined,
}

#[derive(Debug, Default, Serialize)]
pub(crate) struct Counts {
    pub total: usize,
    pub supported: usize,
    pub unsupported: usize,
    pub undetermined: usize,
    pub exact_payload_repeats: usize,
}

fn count<'a>(facts: &Value, judgments: impl Iterator<Item = (&'a str, Support)>) -> Result<Counts> {
    let facts = facts.as_array().context("support_facts_invalid")?;
    let mut by_token = BTreeMap::new();
    for (token, support) in judgments {
        if by_token.insert(token, support).is_some() {
            bail!("support_token_duplicate");
        }
    }
    let mut counts = Counts::default();
    let mut payloads = BTreeSet::new();
    for fact in facts {
        let mut payload = fact.as_object().context("support_fact_invalid")?.clone();
        let token = payload.remove("token").context("support_token_missing")?;
        let support = by_token
            .remove(token.as_str().context("support_token_invalid")?)
            .context("support_token_missing")?;
        counts.total += 1;
        match support {
            Support::Supported => counts.supported += 1,
            Support::Unsupported => counts.unsupported += 1,
            Support::Undetermined => counts.undetermined += 1,
        }
        // Exact equality only. Keep sequence, chapter, claim and evidence fields;
        // never deduplicate denominators or claim semantic duplicate detection.
        if !payloads.insert(serde_json::to_string(&payload)?) {
            counts.exact_payload_repeats += 1;
        }
    }
    if !by_token.is_empty() {
        bail!("support_token_unknown");
    }
    Ok(counts)
}

pub(crate) fn report(
    payload: &Value,
    verdicts: &super::JudgeVerdicts,
) -> Result<BTreeMap<String, Counts>> {
    let mut result = BTreeMap::new();
    for (category, rows) in [
        ("characters", &verdicts.extracted_character_verdicts),
        ("relationships", &verdicts.extracted_relationship_verdicts),
        ("world_rules", &verdicts.extracted_world_rule_verdicts),
    ] {
        result.insert(
            category.into(),
            count(
                &payload["extracted"][category],
                rows.iter().map(|row| (row.extracted.as_str(), row.support)),
            )?,
        );
    }
    result.insert(
        "events".into(),
        count(
            &payload["extracted"]["events"],
            verdicts
                .extracted_event_support
                .iter()
                .map(|row| (row.extracted.as_str(), row.support)),
        )?,
    );
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn raw_counts_preserve_repeats_and_coordinates() {
        let facts = json!([
            {"token":"a", "summary":"given", "sequence":1, "chapter_numbers":[1]},
            {"token":"b", "summary":"given", "sequence":1, "chapter_numbers":[1]},
            {"token":"c", "summary":"given", "sequence":2, "chapter_numbers":[1]},
            {"token":"d", "summary":"given", "sequence":1, "chapter_numbers":[2]}
        ]);
        let rows = [
            ("a", Support::Supported),
            ("b", Support::Unsupported),
            ("c", Support::Undetermined),
            ("d", Support::Supported),
        ];
        let result = count(&facts, rows.into_iter()).unwrap();
        assert_eq!(result.total, 4);
        assert_eq!(
            (result.supported, result.unsupported, result.undetermined),
            (2, 1, 1)
        );
        assert_eq!(result.exact_payload_repeats, 1);
        assert!(count(&facts, rows[..3].iter().copied()).is_err());
        assert!(count(&facts, rows.into_iter().chain([("a", Support::Supported)])).is_err());
        assert!(count(
            &facts,
            rows.into_iter().chain([("unknown", Support::Supported)])
        )
        .is_err());
        assert_eq!(count(&json!([]), std::iter::empty()).unwrap().total, 0);
    }
}
