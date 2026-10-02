import { translate as t, useLocale } from '@/shared/lib/i18n';
import { useEffect, useState, type FormEvent } from 'react';
import { useGenerateGameRules, type CreatePlayerEntityInput } from '@/entities/narrative';
import { getApiErrorCode, getApiErrorMessage } from '@/shared/api/client';
import type { GameRuleTemplate, ResolutionMode } from '@/shared/types';

interface PlayerEntryFormProps {
  novelId: string;
  checkpointChapter: number;
  unlockedThroughChapter: number;
  locations: Array<{ id: string; name: string }>;
  isPending: boolean;
  isTimelineLocked: boolean;
  error?: string;
  onCheckpointChange: (chapter: number) => void;
  onSubmit: (input: CreatePlayerEntityInput) => Promise<unknown>;
}

function tokens(value: string) {
  return value.split(/[,，]/).map(token => token.trim()).filter(Boolean);
}

const unspecifiedLocation = '';

export function PlayerEntryForm({
  novelId,
  checkpointChapter,
  unlockedThroughChapter,
  locations,
  isPending,
  isTimelineLocked,
  error,
  onCheckpointChange,
  onSubmit,
}: PlayerEntryFormProps) {
  useLocale();
  const [name, setName] = useState('');
  const [background, setBackground] = useState('');
  const [capabilities, setCapabilities] = useState('');
  const [locationId, setLocationId] = useState<string | null>(locations[0]?.id ?? null);
  const [inventory, setInventory] = useState('');
  const [resolutionMode, setResolutionMode] = useState<ResolutionMode>('narrative');
  const [gameRules, setGameRules] = useState<GameRuleTemplate>();
  const [scores, setScores] = useState<Record<string, number>>({});
  const generateRules = useGenerateGameRules(novelId);
  const assignedPoints = Object.values(scores).reduce((sum, score) => sum + score, 0);
  const scoresValid = Boolean(gameRules
    && Object.keys(scores).length === gameRules.attributes.length
    && gameRules.attributes.every(attribute => {
      const score = scores[attribute.key];
      return Number.isInteger(score)
        && score >= gameRules.minimum_score
        && score <= gameRules.maximum_score;
    }));
  const advancedReady = resolutionMode === 'narrative'
    || Boolean(gameRules && scoresValid && assignedPoints === gameRules.point_budget);
  const controlsLocked = isPending || isTimelineLocked;

  useEffect(() => {
    if (locationId !== null && !locations.some(location => location.id === locationId)) {
      setLocationId(locations[0]?.id ?? null);
    }
  }, [locationId, locations]);

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (isPending || isTimelineLocked) return;
    try {
      await onSubmit({
        checkpoint_chapter: checkpointChapter,
        name: name.trim(),
        background: background.trim(),
        capabilities: tokens(capabilities),
        location_id: locationId,
        inventory: tokens(inventory),
        rules: resolutionMode === 'advanced' && gameRules ? {
          mode: 'advanced',
          canon_model_version: gameRules.canon_model_version,
          template_schema_version: gameRules.schema_version,
          template_prompt_version: gameRules.prompt_version,
          attributes: scores,
          ...(gameRules.series?.binding ? { series_binding: gameRules.series.binding } : {}),
        } : {
          mode: 'narrative',
          canon_model_version: null,
          template_schema_version: null,
          template_prompt_version: null,
          attributes: {},
        },
      });
    } catch {
      // The mutation error is rendered by the parent.
    }
  };

  return (
    <section
      className="surface-card mt-8 p-6"
      aria-labelledby="player-entry-title"
    >
      <h2 id="player-entry-title" className="text-xl font-semibold text-[#1f1f1f]">
        {t("Create your original character")}
      </h2>
      <p className="mt-2 text-sm text-[#5f6368]">
        {t("You can keep reading before choosing an unlocked entry chapter. Your entry point and earlier history become fixed. You can complete this chapter's fate choice; later events advance through your open-world actions.")}
      </p>
      <form className="mt-5 space-y-4" onSubmit={submit}>
        <label className="block text-sm font-medium text-[#3c4043]">
          {t("Entry chapter")}
          <select
            className="field-control mt-1"
            value={checkpointChapter}
            disabled={controlsLocked}
            onChange={event => onCheckpointChange(Number(event.target.value))}
          >
            {Array.from({ length: unlockedThroughChapter }, (_, index) => index + 1).map(chapter => (
              <option key={chapter} value={chapter}>{t('Chapter {p0}', { p0: chapter })}</option>
            ))}
          </select>
        </label>
        <fieldset disabled={controlsLocked} className="rounded-xl border border-[#d2e3fc] bg-[#f8faff] p-4">
          <legend className="px-1 text-sm font-semibold text-[#0b57d0]">{t("Action checks (advanced)")}</legend>
          <label className="mt-2 flex items-start gap-2 text-sm text-[#3c4043]">
            <input
              type="checkbox"
              checked={resolutionMode === 'advanced'}
              onChange={event => setResolutionMode(event.target.checked ? 'advanced' : 'narrative')}
            />
            <span>
              {t("Enable novel-specific D20 attributes and checks")}
              <span className="mt-1 block text-xs text-[#5f6368]">{t("Narrative mode remains the default. Players of the same novel share the rule template.")}</span>
            </span>
          </label>
          {resolutionMode === 'advanced' ? (
            <div className="mt-4 space-y-3">
              {!gameRules ? (
                <button
                  type="button"
                  className="rounded-lg border border-[#0b57d0] px-4 py-2 text-sm font-medium text-[#0b57d0] disabled:opacity-50"
                  disabled={generateRules.isPending}
                  onClick={() => {
                    generateRules.mutate(undefined, {
                      onSuccess: template => {
                        setGameRules(template);
                        setScores(Object.fromEntries(
                          template.attributes.map(attribute => [attribute.key, attribute.default_score]),
                        ));
                      },
                    });
                  }}
                >
                  {generateRules.isPending ? t("Generating novel rules…") : t("Generate novel-specific rules")}
                </button>
              ) : (
                <>
                  {gameRules.series ? (
                    <div className="rounded-lg bg-[#f8fafd] p-3 text-xs leading-5 text-[#5f6368]">
                      <p className="font-medium text-[#3c4043]">{t("Shared series rules:")}{gameRules.series.name}</p>
                      <p className="mt-1">{gameRules.series.background}</p>
                      <p className="mt-1">
                        {t("Rules come from the series source book. Attribute points, equipment and reading progress stay independent.")}
                      </p>
                    </div>
                  ) : null}
                  <div className="flex items-center justify-between text-xs text-[#5f6368]">
                    <span>{t("Attribute points")} {assignedPoints} / {gameRules.point_budget}</span>
                    <span>{t("D20 · Server checks")}</span>
                  </div>
                  {gameRules.attributes.map(attribute => (
                    <label key={attribute.key} className="grid grid-cols-[1fr_5rem] gap-3 text-sm text-[#3c4043]">
                      <span>
                        <span className="font-medium">{attribute.label}</span>
                        <span className="block text-xs text-[#5f6368]">{attribute.description}</span>
                      </span>
                      <input
                        className="field-control"
                        type="number"
                        min={gameRules.minimum_score}
                        max={gameRules.maximum_score}
                        value={scores[attribute.key] ?? attribute.default_score}
                        onChange={event => setScores(current => ({
                          ...current,
                          [attribute.key]: Number(event.target.value),
                        }))}
                      />
                    </label>
                  ))}
                </>
              )}
              {generateRules.isError ? (
                <p role="alert" className="text-sm text-[#b3261e]">
                  {getApiErrorCode(generateRules.error) === 'game_rule_sources_unavailable'
                    ? t("This novel lacks enough world rules for basic checks. Turn off the advanced option to enter narrative mode.")
                    : getApiErrorCode(generateRules.error) === 'series_rule_source_unavailable'
                      ? t("The series source book's D20 rules are not generated. Use narrative mode first or generate source rules in shelf series management.")
                    : getApiErrorCode(generateRules.error) === 'series_background_pending'
                      ? t("Confirm the shared background in shelf series management before enabling series D20 rules. You can also turn off the advanced option.")
                    : getApiErrorCode(generateRules.error) === 'canon_unavailable'
                      ? t("Parsing is incomplete. Wait for success before generating rules.")
                      : getApiErrorCode(generateRules.error) === 'game_rules_unavailable_at_progress'
                        ? t("Novel rules reference locked chapters. Keep reading and retry, or turn off the advanced option.")
                        : getApiErrorMessage(generateRules.error, t("Novel rule generation failed. Try again later."))}
                </p>
              ) : null}
              {gameRules && !advancedReady ? (
                <p role="alert" className="text-sm text-[#b3261e]">
                  {t('Attributes must be integers from {p0} to {p1}, totaling {p2} points.', { p0: gameRules.minimum_score, p1: gameRules.maximum_score, p2: gameRules.point_budget })}
                </p>
              ) : null}
            </div>
          ) : null}
        </fieldset>
        <label className="block text-sm font-medium text-[#3c4043]">
          {t("Name")}
          <input
            className="field-control mt-1"
            value={name}
            disabled={controlsLocked}
            onChange={event => setName(event.target.value)}
            maxLength={100}
            required
          />
        </label>
        <label className="block text-sm font-medium text-[#3c4043]">
          {t("Background")}
          <textarea
            className="field-control mt-1"
            value={background}
            disabled={controlsLocked}
            onChange={event => setBackground(event.target.value)}
            maxLength={2000}
            rows={3}
            required
          />
        </label>
        <label className="block text-sm font-medium text-[#3c4043]">
          {t("Abilities (comma-separated)")}
          <input
            className="field-control mt-1"
            value={capabilities}
            disabled={controlsLocked}
            onChange={event => setCapabilities(event.target.value)}
            maxLength={3200}
            required
          />
        </label>
        <label className="block text-sm font-medium text-[#3c4043]">
          {t("Starting location (optional)")}
          <select
            className="field-control mt-1"
            value={locationId ?? unspecifiedLocation}
            disabled={controlsLocked}
            onChange={event => setLocationId(
              event.target.value === unspecifiedLocation ? null : event.target.value,
            )}
          >
            <option value={unspecifiedLocation}>{t("Leave unspecified")}</option>
            {locations.map(location => (
              <option key={location.id} value={location.id}>{location.name}</option>
            ))}
          </select>
          <span className="mt-1 block text-xs font-normal text-[#5f6368]">
            {t("Choose a location already seen in the entry chapter, or leave it unspecified.")}
          </span>
        </label>
        <label className="block text-sm font-medium text-[#3c4043]">
          {t("Inventory (optional, comma-separated)")}
          <input
            className="field-control mt-1"
            value={inventory}
            disabled={controlsLocked}
            onChange={event => setInventory(event.target.value)}
            maxLength={6400}
          />
        </label>
        {error ? <p role="alert" className="text-sm text-[#b3261e]">{error}</p> : null}
        <button
          type="submit"
          disabled={controlsLocked || !advancedReady}
          className="primary-action"
        >
          {isPending ? t("Entering the world…") : t("Enter the story")}
        </button>
      </form>
    </section>
  );
}
