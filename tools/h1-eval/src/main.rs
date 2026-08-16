//! Horizon 1 extraction quality gate (recorded mode).
//!
//! This tool judges the checked-in `h1-extraction-v1` corpus against the
//! `extraction-quality-v1` policy (docs/EXTRACTION_QUALITY.md) without
//! writing runtime data. Recorded mode is deterministic and required in CI:
//!
//! ```bash
//! cargo run -p h1-eval -- --recorded --git-sha "$(git rev-parse HEAD)"
//! ```
//!
//! It proves the production import-success gates (document extraction and
//! chapter splitting on zh/en UTF-8/GBK/UTF-16 inputs), the labeled
//! malformed/unsupported error contract, and the curated rubric calibration.
//! It does not claim that any provider meets the semantic quality
//! thresholds; live scoring against a provider is a separate reviewed slice.
//!
//! `--self-test` mutates the corpus and thresholds in memory and proves the
//! gate fails closed on each weakening.

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
};

use anyhow::{bail, ensure, Context, Result};
use novel_service::{
    domain::{
        ports::{DocumentExtractionError, DocumentTextExtractor},
        services::novel_parser::NovelParserService,
    },
    infrastructure::document::EbookTextExtractor,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const CORPUS: &str = include_str!("../corpus/v1.json");
const CORPUS_VERSION: &str = "h1-extraction-v1";
const RUBRIC_VERSION: &str = "h1-extraction-rubric-v1";
const MAX_CORPUS_BYTES: usize = 512 * 1024;
const OVERSIZED_TEXT_BYTES: usize = 10 * 1024 * 1024 + 1;

const REQUIRED_THRESHOLDS: Thresholds = Thresholds {
    import_success_basis_points: 10_000,
    recall_basis_points: 800,
    precision_basis_points: 800,
    max_hallucination_basis_points: 200,
    max_chronology_violations: 0,
    provenance_basis_points: 10_000,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Thresholds {
    import_success_basis_points: u16,
    recall_basis_points: u16,
    precision_basis_points: u16,
    max_hallucination_basis_points: u16,
    max_chronology_violations: u16,
    provenance_basis_points: u16,
}

impl Thresholds {
    fn at_least_required(&self) -> bool {
        self.import_success_basis_points >= REQUIRED_THRESHOLDS.import_success_basis_points
            && self.recall_basis_points >= REQUIRED_THRESHOLDS.recall_basis_points
            && self.precision_basis_points >= REQUIRED_THRESHOLDS.precision_basis_points
            && self.max_hallucination_basis_points
                <= REQUIRED_THRESHOLDS.max_hallucination_basis_points
            // REQUIRED_THRESHOLDS.max_chronology_violations is zero, so the
            // corpus must require exactly zero; `==` keeps clippy's
            // absurd-comparison lint quiet without changing the semantics.
            && self.max_chronology_violations == REQUIRED_THRESHOLDS.max_chronology_violations
            && self.provenance_basis_points >= REQUIRED_THRESHOLDS.provenance_basis_points
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    schema_version: u8,
    corpus_version: String,
    rubric_version: String,
    thresholds: Thresholds,
    positive_cases: Vec<PositiveCase>,
    negative_cases: Vec<NegativeCase>,
    calibration_cases: Vec<CalibrationCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PositiveCase {
    id: String,
    language: String,
    encodings: Vec<String>,
    file_name: String,
    content_type: String,
    chapters: Vec<String>,
    anchors_first_chapter: Vec<String>,
    anchors_anywhere: Vec<String>,
    expected: ExpectedFacts,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedFacts {
    characters: Vec<String>,
    relationships: Vec<String>,
    events: Vec<String>,
    world_rules: Vec<String>,
    chronology_pairs: Vec<(String, String)>,
    provenance_required: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptedFacts {
    characters: Vec<String>,
    relationships: Vec<String>,
    events: Vec<String>,
    world_rules: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum NegativeKind {
    InvalidUtf16Text,
    EmptyText,
    UnsupportedExtension,
    OversizedText,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ExpectedError {
    InvalidTextEncoding,
    EmptyDocument,
    UnsupportedType,
    UploadTooLarge,
}

impl ExpectedError {
    fn matches(&self, error: &DocumentExtractionError) -> bool {
        matches!(
            (self, error),
            (
                Self::InvalidTextEncoding,
                DocumentExtractionError::InvalidTextEncoding
            ) | (Self::EmptyDocument, DocumentExtractionError::EmptyDocument)
                | (
                    Self::UnsupportedType,
                    DocumentExtractionError::UnsupportedType
                )
                | (
                    Self::UploadTooLarge,
                    DocumentExtractionError::UploadTooLarge { .. }
                )
        )
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NegativeCase {
    id: String,
    kind: NegativeKind,
    expected_error: ExpectedError,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CalibrationCase {
    id: String,
    expected: ExpectedFacts,
    accepted: AcceptedFacts,
    chronology_violations: u16,
    provenance_missing: u16,
    expected_pass: bool,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct CaseOutcome {
    pass: bool,
    detail: String,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct Report {
    tool: &'static str,
    git_sha: String,
    corpus_version: String,
    rubric_version: String,
    thresholds: Thresholds,
    structural_cases: BTreeMap<String, CaseOutcome>,
    calibration_cases: BTreeMap<String, CaseOutcome>,
    totals: Totals,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct Totals {
    structural_pass: usize,
    structural_total: usize,
    calibration_pass: usize,
    calibration_total: usize,
}

fn validate_git_sha(sha: &str) -> Result<()> {
    ensure!(
        sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()),
        "git-sha must be a 40-character commit SHA"
    );
    Ok(())
}

fn validate_corpus(corpus: &Corpus) -> Result<()> {
    ensure!(
        CORPUS.len() <= MAX_CORPUS_BYTES,
        "corpus exceeds the byte cap"
    );
    ensure!(
        corpus.schema_version == 1,
        "unsupported corpus schema version"
    );
    ensure!(
        corpus.corpus_version == CORPUS_VERSION,
        "unsupported corpus version"
    );
    ensure!(
        corpus.rubric_version == RUBRIC_VERSION,
        "unsupported rubric version"
    );
    ensure!(
        corpus.thresholds.at_least_required(),
        "corpus thresholds weaken the approved extraction-quality-v1 policy"
    );
    ensure!(!corpus.positive_cases.is_empty(), "no positive cases");
    ensure!(!corpus.negative_cases.is_empty(), "no negative cases");
    ensure!(!corpus.calibration_cases.is_empty(), "no calibration cases");

    let mut ids = BTreeSet::new();
    let mut register = |id: &str| -> Result<()> {
        ensure!(ids.insert(id.to_owned()), "duplicate case id {id}");
        Ok(())
    };
    for case in &corpus.positive_cases {
        register(&case.id)?;
        ensure!(
            matches!(case.language.as_str(), "zh" | "en"),
            "{} declares an unsupported language slice",
            case.id
        );
        ensure!(!case.chapters.is_empty(), "{} has no chapters", case.id);
        ensure!(
            !case.encodings.is_empty(),
            "{} declares no encodings",
            case.id
        );
        // Anti-vacuity: every expected-fact category must be non-empty so an
        // empty accepted canon can never pass the gate.
        ensure!(
            !case.expected.characters.is_empty()
                && !case.expected.relationships.is_empty()
                && !case.expected.events.is_empty()
                && !case.expected.world_rules.is_empty(),
            "{} has an empty expected-fact category",
            case.id
        );
        ensure!(
            !case.expected.chronology_pairs.is_empty(),
            "{} has no chronology constraints",
            case.id
        );
        ensure!(
            case.expected.provenance_required,
            "{} waives provenance",
            case.id
        );
    }
    for case in &corpus.negative_cases {
        register(&case.id)?;
    }
    for case in &corpus.calibration_cases {
        register(&case.id)?;
        ensure!(
            !case.expected.characters.is_empty() && !case.accepted.characters.is_empty(),
            "{} calibration is vacuous",
            case.id
        );
    }
    Ok(())
}

fn encode(text: &str, encoding: &str) -> Result<Vec<u8>> {
    match encoding {
        "utf8" => Ok(text.as_bytes().to_vec()),
        "utf16le" => {
            let mut bytes = vec![0xFF, 0xFE];
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            Ok(bytes)
        }
        "gbk" => {
            let (encoded, _, had_errors) = encoding_rs::GBK.encode(text);
            ensure!(!had_errors, "GBK encoding failed for corpus text");
            Ok(encoded.into_owned())
        }
        other => bail!("unsupported corpus encoding {other}"),
    }
}

fn structural_case(case: &PositiveCase) -> Result<CaseOutcome> {
    let source = case.chapters.join("\n\n");
    for encoding in &case.encodings {
        let bytes = encode(&source, encoding)?;
        let text = EbookTextExtractor
            .extract_text(Some(&case.file_name), Some(&case.content_type), &bytes)
            .with_context(|| format!("{} extraction failed for {encoding}", case.id))?;
        let chapters = NovelParserService::parse_chapters(Uuid::new_v4(), &text)
            .with_context(|| format!("{} split failed for {encoding}", case.id))?;
        ensure!(
            chapters.len() >= 2,
            "{} produced fewer than two chapters for {encoding}",
            case.id
        );
        for (index, chapter) in chapters.iter().enumerate() {
            ensure!(
                chapter.chapter_number == index as i32 + 1,
                "{} chapter numbering is not contiguous for {encoding}",
                case.id
            );
            ensure!(
                !chapter.content.trim().is_empty(),
                "{} has a blank chapter for {encoding}",
                case.id
            );
        }
        let first = &chapters[0].content;
        for anchor in &case.anchors_first_chapter {
            ensure!(
                first.contains(anchor),
                "{} first chapter lacks anchor {anchor:?} for {encoding}",
                case.id
            );
        }
        for anchor in &case.anchors_anywhere {
            ensure!(
                text.contains(anchor),
                "{} text lacks anchor {anchor:?} for {encoding}",
                case.id
            );
        }
    }
    Ok(CaseOutcome {
        pass: true,
        detail: format!("{} encodings passed", case.encodings.len()),
    })
}

fn negative_case(case: &NegativeCase) -> Result<CaseOutcome> {
    let (file_name, content_type, data) = match case.kind {
        NegativeKind::InvalidUtf16Text => (
            Some("story.txt"),
            Some("text/plain"),
            vec![0xFF, 0xFE, 0x61, 0x00, 0x62],
        ),
        NegativeKind::EmptyText => (Some("story.txt"), Some("text/plain"), b"  \n\n  ".to_vec()),
        NegativeKind::UnsupportedExtension => (
            Some("story.docx"),
            Some("application/octet-stream"),
            b"PK\x03\x04not-a-zip".to_vec(),
        ),
        NegativeKind::OversizedText => (
            Some("story.txt"),
            Some("text/plain"),
            vec![b'a'; OVERSIZED_TEXT_BYTES],
        ),
    };
    let error = EbookTextExtractor
        .extract_text(file_name, content_type, &data)
        .expect_err("negative case must be rejected");
    ensure!(
        case.expected_error.matches(&error),
        "{} produced {:?}, expected {:?}",
        case.id,
        error,
        case.expected_error
    );
    Ok(CaseOutcome {
        pass: true,
        detail: format!("rejected with {error}"),
    })
}

struct CategoryScores {
    recall_bp: u16,
    precision_bp: u16,
    hallucination_bp: u16,
}

fn score_category(expected: &[String], accepted: &[String]) -> CategoryScores {
    let expected_set: BTreeSet<&str> = expected.iter().map(String::as_str).collect();
    let accepted_set: BTreeSet<&str> = accepted.iter().map(String::as_str).collect();
    let matched = expected_set.intersection(&accepted_set).count();
    let recall_bp = (matched * 10_000) / expected_set.len();
    let (precision_bp, hallucination_bp) = if accepted_set.is_empty() {
        (0, 0)
    } else {
        let precision_bp = (matched * 10_000) / accepted_set.len();
        (precision_bp, 10_000 - precision_bp)
    };
    CategoryScores {
        recall_bp: recall_bp as u16,
        precision_bp: precision_bp as u16,
        hallucination_bp: hallucination_bp as u16,
    }
}

fn calibration_case(case: &CalibrationCase, thresholds: &Thresholds) -> Result<CaseOutcome> {
    let categories: [(&str, &[String], &[String]); 4] = [
        (
            "characters",
            &case.expected.characters,
            &case.accepted.characters,
        ),
        (
            "relationships",
            &case.expected.relationships,
            &case.accepted.relationships,
        ),
        ("events", &case.expected.events, &case.accepted.events),
        (
            "world_rules",
            &case.expected.world_rules,
            &case.accepted.world_rules,
        ),
    ];
    let mut weak = Vec::new();
    let mut empty_canon = true;
    for (name, expected, accepted) in categories {
        let scores = score_category(expected, accepted);
        if !accepted.is_empty() {
            empty_canon = false;
        }
        if scores.recall_bp < thresholds.recall_basis_points {
            weak.push(format!("{name} recall {}bp", scores.recall_bp));
        }
        if scores.precision_bp < thresholds.precision_basis_points {
            weak.push(format!("{name} precision {}bp", scores.precision_bp));
        }
        if scores.hallucination_bp > thresholds.max_hallucination_basis_points {
            weak.push(format!(
                "{name} hallucination {}bp",
                scores.hallucination_bp
            ));
        }
    }
    if case.chronology_violations > thresholds.max_chronology_violations {
        weak.push(format!(
            "{} chronology violations",
            case.chronology_violations
        ));
    }
    if case.provenance_missing > 0 {
        weak.push(format!(
            "{} facts without provenance",
            case.provenance_missing
        ));
    }
    if empty_canon {
        weak.push("accepted canon is empty".into());
    }
    let pass = weak.is_empty();
    ensure!(
        pass == case.expected_pass,
        "{} verdict mismatch: expected {} got {} ({})",
        case.id,
        case.expected_pass,
        pass,
        weak.join(", ")
    );
    Ok(CaseOutcome {
        // `pass` records that the verdict matched the curated expectation,
        // not that the case is within thresholds (adversarial cases are not).
        pass: true,
        detail: if weak.is_empty() {
            "verdict within thresholds as curated".into()
        } else {
            format!("verdict rejects as curated: {}", weak.join(", "))
        },
    })
}

fn run_recorded(git_sha: &str) -> Result<Report> {
    validate_git_sha(git_sha)?;
    let corpus: Corpus = serde_json::from_str(CORPUS).context("corpus is not valid JSON")?;
    validate_corpus(&corpus)?;

    let mut structural_cases = BTreeMap::new();
    for case in &corpus.positive_cases {
        let outcome = structural_case(case).with_context(|| format!("case {}", case.id))?;
        structural_cases.insert(case.id.clone(), outcome);
    }
    for case in &corpus.negative_cases {
        let outcome = negative_case(case).with_context(|| format!("case {}", case.id))?;
        structural_cases.insert(case.id.clone(), outcome);
    }
    ensure!(
        structural_cases.values().all(|outcome| outcome.pass),
        "structural import-success gate failed"
    );

    let mut calibration_cases = BTreeMap::new();
    for case in &corpus.calibration_cases {
        let outcome = calibration_case(case, &corpus.thresholds)
            .with_context(|| format!("case {}", case.id))?;
        calibration_cases.insert(case.id.clone(), outcome);
    }

    Ok(Report {
        tool: "h1-eval",
        git_sha: git_sha.to_owned(),
        corpus_version: corpus.corpus_version,
        rubric_version: corpus.rubric_version,
        thresholds: corpus.thresholds,
        totals: Totals {
            structural_pass: structural_cases.len(),
            structural_total: structural_cases.len(),
            calibration_pass: calibration_cases.len(),
            calibration_total: calibration_cases.len(),
        },
        structural_cases,
        calibration_cases,
    })
}

fn self_test() -> Result<()> {
    // Threshold weakening must fail closed.
    let mut corpus: Corpus = serde_json::from_str(CORPUS)?;
    validate_corpus(&corpus)?;
    corpus.thresholds.recall_basis_points = REQUIRED_THRESHOLDS.recall_basis_points - 1;
    ensure!(
        validate_corpus(&corpus).is_err(),
        "self-test failed: lowered thresholds passed"
    );

    // An emptied expected-fact category must fail closed (anti-vacuity).
    let mut corpus: Corpus = serde_json::from_str(CORPUS)?;
    corpus.positive_cases[0].expected.characters.clear();
    ensure!(
        validate_corpus(&corpus).is_err(),
        "self-test failed: emptied category passed"
    );

    // A hallucinated fact beyond the approved bound must fail the verdict.
    let mut corpus: Corpus = serde_json::from_str(CORPUS)?;
    corpus.calibration_cases[1].expected_pass = true;
    ensure!(
        calibration_case(&corpus.calibration_cases[1], &corpus.thresholds).is_err(),
        "self-test failed: hallucinated fact passed"
    );

    // A chronology violation must fail the verdict.
    let mut corpus: Corpus = serde_json::from_str(CORPUS)?;
    corpus.calibration_cases[2].expected_pass = true;
    ensure!(
        calibration_case(&corpus.calibration_cases[2], &corpus.thresholds).is_err(),
        "self-test failed: chronology violation passed"
    );

    // Non-commit SHAs must fail closed.
    ensure!(
        validate_git_sha("not-a-sha").is_err(),
        "self-test failed: invalid SHA accepted"
    );

    // Duplicate case ids must fail closed.
    let mut corpus: Corpus = serde_json::from_str(CORPUS)?;
    corpus.negative_cases[0].id = corpus.positive_cases[0].id.clone();
    ensure!(
        validate_corpus(&corpus).is_err(),
        "self-test failed: duplicate ids passed"
    );

    println!("h1-eval self-test passed");
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.as_slice() {
        [mode, flag, sha] if mode == "--recorded" && flag == "--git-sha" => {
            let report = run_recorded(sha)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        [mode] if mode == "--self-test" => self_test(),
        _ => bail!("usage: h1-eval --recorded --git-sha <sha> | h1-eval --self-test"),
    }
}
