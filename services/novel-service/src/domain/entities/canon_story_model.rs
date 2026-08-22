use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const CANON_STORY_SCHEMA_VERSION: i32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonStoryModel {
    pub id: Uuid,
    pub novel_id: Uuid,
    pub model_version: i32,
    pub schema_version: i32,
    pub prompt_version: String,
    pub content: CanonStoryContent,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonStoryContent {
    pub arcs: Vec<StoryArc>,
    pub events: Vec<CanonEvent>,
    pub locations: Vec<CanonLocation>,
    pub factions: Vec<CanonFaction>,
    pub world_rules: Vec<WorldRule>,
    pub character_goals: Vec<CharacterGoal>,
    pub relationships: Vec<CanonRelationship>,
    pub deaths: Vec<CanonDeath>,
    pub unresolved_threads: Vec<UnresolvedThread>,
    pub ending: CanonEndingSnapshot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceEvidence {
    pub provenance: Vec<SourceCitation>,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceCitation {
    pub chapter_number: i32,
    pub excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryArc {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub event_ids: Vec<String>,
    pub evidence: SourceEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonEvent {
    pub id: String,
    pub sequence: i32,
    pub summary: String,
    pub caused_by: Vec<String>,
    pub location_ids: Vec<String>,
    pub character_ids: Vec<Uuid>,
    pub faction_ids: Vec<String>,
    pub evidence: SourceEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonLocation {
    pub id: String,
    pub name: String,
    pub description: String,
    pub evidence: SourceEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonFaction {
    pub id: String,
    pub name: String,
    pub description: String,
    pub evidence: SourceEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldRule {
    pub id: String,
    pub description: String,
    pub hard: bool,
    pub evidence: SourceEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterGoal {
    pub id: String,
    pub character_id: Uuid,
    pub description: String,
    pub evidence: SourceEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonRelationship {
    pub id: String,
    pub from_character_id: Uuid,
    pub to_character_id: Uuid,
    pub kind: String,
    pub description: String,
    pub evidence: SourceEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonDeath {
    pub id: String,
    pub character_id: Uuid,
    pub event_id: String,
    pub description: String,
    pub evidence: SourceEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnresolvedThread {
    pub id: String,
    pub description: String,
    pub evidence: SourceEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonEndingSnapshot {
    pub summary: String,
    pub character_states: BTreeMap<Uuid, String>,
    pub faction_states: BTreeMap<String, String>,
    pub location_states: BTreeMap<String, String>,
    pub unresolved_thread_ids: Vec<String>,
    pub evidence: SourceEvidence,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid canon story model: {0}")]
pub struct CanonStoryModelError(String);

impl CanonStoryModel {
    pub fn validate(
        &self,
        source_chapters: &BTreeMap<i32, String>,
        canonical_character_ids: &HashSet<Uuid>,
    ) -> Result<(), CanonStoryModelError> {
        if self.id.is_nil() || self.novel_id.is_nil() {
            return invalid("model and novel IDs must not be nil");
        }
        if self.model_version < 1 {
            return invalid("model_version must be at least 1");
        }
        if self.schema_version != CANON_STORY_SCHEMA_VERSION {
            return invalid(format!(
                "schema_version must be {CANON_STORY_SCHEMA_VERSION}"
            ));
        }
        validate_token("prompt_version", &self.prompt_version, 100)?;
        if source_chapters.is_empty() {
            return invalid("source novel must contain at least one chapter");
        }
        let chapter_count = i32::try_from(source_chapters.len())
            .map_err(|_| error("source novel has too many chapters"))?;
        if !source_chapters.keys().copied().eq(1..=chapter_count) {
            return invalid("source chapters must be contiguous from chapter 1");
        }
        if self.content.arcs.is_empty() || self.content.events.is_empty() {
            return invalid("arcs and events must not be empty");
        }
        if canonical_character_ids.is_empty() {
            return invalid("canonical story models require at least one character");
        }

        validate_unique_ids("arc", self.content.arcs.iter().map(|item| item.id.as_str()))?;
        validate_unique_ids(
            "event",
            self.content.events.iter().map(|item| item.id.as_str()),
        )?;
        validate_unique_ids(
            "location",
            self.content.locations.iter().map(|item| item.id.as_str()),
        )?;
        validate_unique_ids(
            "faction",
            self.content.factions.iter().map(|item| item.id.as_str()),
        )?;
        validate_unique_ids(
            "world rule",
            self.content.world_rules.iter().map(|item| item.id.as_str()),
        )?;
        validate_unique_ids(
            "character goal",
            self.content
                .character_goals
                .iter()
                .map(|item| item.id.as_str()),
        )?;
        validate_unique_ids(
            "relationship",
            self.content
                .relationships
                .iter()
                .map(|item| item.id.as_str()),
        )?;
        validate_unique_ids(
            "death",
            self.content.deaths.iter().map(|item| item.id.as_str()),
        )?;
        validate_unique_ids(
            "unresolved thread",
            self.content
                .unresolved_threads
                .iter()
                .map(|item| item.id.as_str()),
        )?;

        let event_sequences: HashMap<&str, i32> = self
            .content
            .events
            .iter()
            .map(|event| (event.id.as_str(), event.sequence))
            .collect();
        let location_ids: HashSet<&str> = self
            .content
            .locations
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        let faction_ids: HashSet<&str> = self
            .content
            .factions
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        let thread_ids: HashSet<&str> = self
            .content
            .unresolved_threads
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        let mut covered_events = HashSet::new();

        for (index, event) in self.content.events.iter().enumerate() {
            if event.sequence != index as i32 + 1 {
                return invalid("event sequence must be contiguous and match list order");
            }
            validate_text("event summary", &event.summary, 10_000)?;
            validate_evidence(&event.evidence, source_chapters)?;
            let mut causes = HashSet::new();
            for cause in &event.caused_by {
                if !causes.insert(cause.as_str()) {
                    return invalid(format!("event {} has duplicate cause {cause}", event.id));
                }
                let cause_sequence = event_sequences.get(cause.as_str()).ok_or_else(|| {
                    error(format!("event {} has an unknown cause {cause}", event.id))
                })?;
                if *cause_sequence >= event.sequence {
                    return invalid(format!(
                        "event {} must only depend on earlier events",
                        event.id
                    ));
                }
            }
            require_known_refs("location", &event.id, &event.location_ids, &location_ids)?;
            require_known_refs("faction", &event.id, &event.faction_ids, &faction_ids)?;
            require_known_characters(&event.id, &event.character_ids, canonical_character_ids)?;
        }

        for arc in &self.content.arcs {
            validate_text("arc title", &arc.title, 500)?;
            validate_text("arc summary", &arc.summary, 10_000)?;
            validate_evidence(&arc.evidence, source_chapters)?;
            if arc.event_ids.is_empty() {
                return invalid(format!("arc {} must contain at least one event", arc.id));
            }
            let mut previous_sequence = 0;
            for event_id in &arc.event_ids {
                let sequence = event_sequences.get(event_id.as_str()).ok_or_else(|| {
                    error(format!("arc {} has an unknown event {event_id}", arc.id))
                })?;
                if *sequence <= previous_sequence {
                    return invalid(format!("arc {} events must be ordered", arc.id));
                }
                previous_sequence = *sequence;
                covered_events.insert(event_id.as_str());
            }
        }
        if covered_events.len() != self.content.events.len() {
            return invalid("every event must belong to at least one arc");
        }

        for location in &self.content.locations {
            validate_text("location name", &location.name, 500)?;
            validate_text("location description", &location.description, 10_000)?;
            validate_evidence(&location.evidence, source_chapters)?;
        }
        for faction in &self.content.factions {
            validate_text("faction name", &faction.name, 500)?;
            validate_text("faction description", &faction.description, 10_000)?;
            validate_evidence(&faction.evidence, source_chapters)?;
        }
        for rule in &self.content.world_rules {
            validate_text("world rule", &rule.description, 10_000)?;
            validate_evidence(&rule.evidence, source_chapters)?;
        }
        for goal in &self.content.character_goals {
            require_known_character(&goal.id, goal.character_id, canonical_character_ids)?;
            validate_text("character goal", &goal.description, 10_000)?;
            validate_evidence(&goal.evidence, source_chapters)?;
        }
        for relationship in &self.content.relationships {
            require_known_character(
                &relationship.id,
                relationship.from_character_id,
                canonical_character_ids,
            )?;
            require_known_character(
                &relationship.id,
                relationship.to_character_id,
                canonical_character_ids,
            )?;
            if relationship.from_character_id == relationship.to_character_id {
                return invalid(format!(
                    "relationship {} cannot reference the same character twice",
                    relationship.id
                ));
            }
            validate_token("relationship kind", &relationship.kind, 500)?;
            validate_text(
                "relationship description",
                &relationship.description,
                10_000,
            )?;
            validate_evidence(&relationship.evidence, source_chapters)?;
        }
        for death in &self.content.deaths {
            require_known_character(&death.id, death.character_id, canonical_character_ids)?;
            let event = self
                .content
                .events
                .iter()
                .find(|event| event.id == death.event_id)
                .ok_or_else(|| {
                    error(format!(
                        "death {} has an unknown event {}",
                        death.id, death.event_id
                    ))
                })?;
            if !event.character_ids.contains(&death.character_id) {
                return invalid(format!(
                    "death {} character must participate in its event",
                    death.id
                ));
            }
            validate_text("death description", &death.description, 10_000)?;
            validate_evidence(&death.evidence, source_chapters)?;
        }
        for thread in &self.content.unresolved_threads {
            validate_text("unresolved thread", &thread.description, 10_000)?;
            validate_evidence(&thread.evidence, source_chapters)?;
        }

        let ending = &self.content.ending;
        validate_text("ending summary", &ending.summary, 20_000)?;
        validate_evidence(&ending.evidence, source_chapters)?;
        for character_id in ending.character_states.keys() {
            require_known_character("ending snapshot", *character_id, canonical_character_ids)?;
        }
        for (faction_id, state) in &ending.faction_states {
            if !faction_ids.contains(faction_id.as_str()) {
                return invalid(format!("ending snapshot has unknown faction {faction_id}"));
            }
            validate_text("ending faction state", state, 10_000)?;
        }
        for (location_id, state) in &ending.location_states {
            if !location_ids.contains(location_id.as_str()) {
                return invalid(format!(
                    "ending snapshot has unknown location {location_id}"
                ));
            }
            validate_text("ending location state", state, 10_000)?;
        }
        require_known_refs(
            "unresolved thread",
            "ending snapshot",
            &ending.unresolved_thread_ids,
            &thread_ids,
        )?;
        for state in ending.character_states.values() {
            validate_text("ending character state", state, 10_000)?;
        }

        Ok(())
    }
}

fn validate_unique_ids<'a>(
    kind: &str,
    ids: impl Iterator<Item = &'a str>,
) -> Result<(), CanonStoryModelError> {
    let mut seen = HashSet::new();
    for id in ids {
        validate_token(&format!("{kind} id"), id, 100)?;
        if !seen.insert(id) {
            return invalid(format!("duplicate {kind} id {id}"));
        }
    }
    Ok(())
}

fn validate_evidence(
    evidence: &SourceEvidence,
    source_chapters: &BTreeMap<i32, String>,
) -> Result<(), CanonStoryModelError> {
    if !evidence.confidence.is_finite() || !(0.0..=1.0).contains(&evidence.confidence) {
        return invalid("evidence confidence must be between 0 and 1");
    }
    if evidence.provenance.is_empty() {
        return invalid("every canonical item must have source provenance");
    }
    for citation in &evidence.provenance {
        let source = source_chapters
            .get(&citation.chapter_number)
            .ok_or_else(|| {
                error(format!(
                    "source chapter {} does not exist",
                    citation.chapter_number
                ))
            })?;
        validate_text("source excerpt", &citation.excerpt, 2_000)?;
        if !source.contains(&citation.excerpt) {
            return invalid(format!(
                "source excerpt does not occur in chapter {}",
                citation.chapter_number
            ));
        }
    }
    Ok(())
}

fn validate_text(name: &str, value: &str, max_chars: usize) -> Result<(), CanonStoryModelError> {
    let length = value.chars().count();
    if value.trim().is_empty()
        || length > max_chars
        || value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return invalid(format!(
            "{name} must contain 1-{max_chars} printable characters"
        ));
    }
    Ok(())
}

fn validate_token(name: &str, value: &str, max_chars: usize) -> Result<(), CanonStoryModelError> {
    validate_text(name, value, max_chars)?;
    if value.trim() != value || value.chars().any(char::is_control) {
        return invalid(format!(
            "{name} must not contain surrounding whitespace or control characters"
        ));
    }
    Ok(())
}

fn require_known_refs(
    kind: &str,
    owner: &str,
    ids: &[String],
    known: &HashSet<&str>,
) -> Result<(), CanonStoryModelError> {
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(id.as_str()) {
            return invalid(format!("{owner} has duplicate {kind} {id}"));
        }
        if !known.contains(id.as_str()) {
            return invalid(format!("{owner} has unknown {kind} {id}"));
        }
    }
    Ok(())
}

fn require_known_characters(
    owner: &str,
    ids: &[Uuid],
    known: &HashSet<Uuid>,
) -> Result<(), CanonStoryModelError> {
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(*id) {
            return invalid(format!("{owner} has duplicate character {id}"));
        }
        require_known_character(owner, *id, known)?;
    }
    Ok(())
}

fn require_known_character(
    owner: &str,
    id: Uuid,
    known: &HashSet<Uuid>,
) -> Result<(), CanonStoryModelError> {
    if id.is_nil() || !known.contains(&id) {
        return invalid(format!("{owner} has unknown character {id}"));
    }
    Ok(())
}

fn error(message: impl Into<String>) -> CanonStoryModelError {
    CanonStoryModelError(message.into())
}

fn invalid<T>(message: impl Into<String>) -> Result<T, CanonStoryModelError> {
    Err(error(message))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence(chapter_number: i32) -> SourceEvidence {
        SourceEvidence {
            provenance: vec![SourceCitation {
                chapter_number,
                excerpt: "source excerpt".into(),
            }],
            confidence: 0.9,
        }
    }

    fn valid_model() -> (CanonStoryModel, HashSet<Uuid>) {
        let protagonist = Uuid::new_v4();
        let antagonist = Uuid::new_v4();
        let model = CanonStoryModel {
            id: Uuid::new_v4(),
            novel_id: Uuid::new_v4(),
            model_version: 1,
            schema_version: CANON_STORY_SCHEMA_VERSION,
            prompt_version: "canon-extraction-v1".into(),
            content: CanonStoryContent {
                arcs: vec![StoryArc {
                    id: "arc-1".into(),
                    title: "Journey".into(),
                    summary: "The complete mainline.".into(),
                    event_ids: vec!["event-1".into(), "event-2".into()],
                    evidence: evidence(1),
                }],
                events: vec![
                    CanonEvent {
                        id: "event-1".into(),
                        sequence: 1,
                        summary: "The journey begins.".into(),
                        caused_by: vec![],
                        location_ids: vec!["location-1".into()],
                        character_ids: vec![protagonist],
                        faction_ids: vec!["faction-1".into()],
                        evidence: evidence(1),
                    },
                    CanonEvent {
                        id: "event-2".into(),
                        sequence: 2,
                        summary: "The antagonist falls.".into(),
                        caused_by: vec!["event-1".into()],
                        location_ids: vec!["location-1".into()],
                        character_ids: vec![protagonist, antagonist],
                        faction_ids: vec!["faction-1".into()],
                        evidence: evidence(2),
                    },
                ],
                locations: vec![CanonLocation {
                    id: "location-1".into(),
                    name: "North Tower".into(),
                    description: "A tower beyond the city.".into(),
                    evidence: evidence(1),
                }],
                factions: vec![CanonFaction {
                    id: "faction-1".into(),
                    name: "Wardens".into(),
                    description: "They protect the tower.".into(),
                    evidence: evidence(1),
                }],
                world_rules: vec![WorldRule {
                    id: "rule-1".into(),
                    description: "The dead remain dead.".into(),
                    hard: true,
                    evidence: evidence(1),
                }],
                character_goals: vec![CharacterGoal {
                    id: "goal-1".into(),
                    character_id: protagonist,
                    description: "Reach the tower.".into(),
                    evidence: evidence(1),
                }],
                relationships: vec![CanonRelationship {
                    id: "relationship-1".into(),
                    from_character_id: protagonist,
                    to_character_id: antagonist,
                    kind: "rivals".into(),
                    description: "They oppose one another.".into(),
                    evidence: evidence(1),
                }],
                deaths: vec![CanonDeath {
                    id: "death-1".into(),
                    character_id: antagonist,
                    event_id: "event-2".into(),
                    description: "The antagonist dies at the tower.".into(),
                    evidence: evidence(2),
                }],
                unresolved_threads: vec![UnresolvedThread {
                    id: "thread-1".into(),
                    description: "The tower's origin is unknown.".into(),
                    evidence: evidence(2),
                }],
                ending: CanonEndingSnapshot {
                    summary: "The journey ends at the tower.".into(),
                    character_states: BTreeMap::from([
                        (protagonist, "alive".into()),
                        (antagonist, "dead".into()),
                    ]),
                    faction_states: BTreeMap::from([("faction-1".into(), "disbanded".into())]),
                    location_states: BTreeMap::from([("location-1".into(), "ruined".into())]),
                    unresolved_thread_ids: vec!["thread-1".into()],
                    evidence: evidence(2),
                },
            },
            created_at: Utc::now(),
        };
        (model, HashSet::from([protagonist, antagonist]))
    }

    #[test]
    fn validates_source_bounds_references_and_causal_order() {
        let (model, characters) = valid_model();
        let chapters = BTreeMap::from([
            (1, "source excerpt in chapter one".into()),
            (2, "source excerpt in chapter two".into()),
        ]);
        model.validate(&chapters, &characters).unwrap();

        let mut future_cause = model.clone();
        future_cause.content.events[0].caused_by = vec!["event-2".into()];
        assert!(future_cause.validate(&chapters, &characters).is_err());

        let mut missing_provenance = model.clone();
        missing_provenance.content.arcs[0]
            .evidence
            .provenance
            .clear();
        assert!(missing_provenance.validate(&chapters, &characters).is_err());

        let mut spoiler = model.clone();
        spoiler.content.events[0].evidence.provenance[0].chapter_number = 3;
        assert!(spoiler.validate(&chapters, &characters).is_err());

        let mut unknown_character = model.clone();
        unknown_character.content.events[0].character_ids = vec![Uuid::new_v4()];
        assert!(unknown_character.validate(&chapters, &characters).is_err());

        let mut partial_ending = model.clone();
        partial_ending.content.ending.character_states.pop_first();
        partial_ending.validate(&chapters, &characters).unwrap();

        let mut unknown_ending_character = model.clone();
        unknown_ending_character
            .content
            .ending
            .character_states
            .insert(Uuid::new_v4(), "unknown".into());
        assert!(unknown_ending_character
            .validate(&chapters, &characters)
            .is_err());

        let mut unknown_field = serde_json::to_value(model).unwrap();
        unknown_field["content"]["events"][0]["invented"] = true.into();
        assert!(serde_json::from_value::<CanonStoryModel>(unknown_field).is_err());
    }
}
