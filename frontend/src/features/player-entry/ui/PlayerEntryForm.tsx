import { useEffect, useState, type FormEvent } from 'react';
import { useGenerateGameRules, type CreatePlayerEntityInput } from '@/entities/narrative/api';
import { getApiErrorMessage } from '@/shared/api/client';
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
  const [name, setName] = useState('');
  const [background, setBackground] = useState('');
  const [capabilities, setCapabilities] = useState('');
  const [locationId, setLocationId] = useState(locations[0]?.id ?? '');
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
    if (!locations.some(location => location.id === locationId)) {
      setLocationId(locations[0]?.id ?? '');
    }
  }, [locationId, locations]);

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (isPending || isTimelineLocked || !locationId) return;
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
        创建你的原创角色
      </h2>
      <p className="mt-2 text-sm text-[#5f6368]">
        你可以先继续阅读，再从已解锁章节选择入场点。创建后入场点及此前历史不可更改；
        你仍可完成入场章的命运选择，之后的因果由你的开放世界行动推进。
      </p>
      <form className="mt-5 space-y-4" onSubmit={submit}>
        <label className="block text-sm font-medium text-[#3c4043]">
          入场章节
          <select
            className="field-control mt-1"
            value={checkpointChapter}
            disabled={controlsLocked}
            onChange={event => onCheckpointChange(Number(event.target.value))}
          >
            {Array.from({ length: unlockedThroughChapter }, (_, index) => index + 1).map(chapter => (
              <option key={chapter} value={chapter}>第 {chapter} 章</option>
            ))}
          </select>
        </label>
        <fieldset disabled={controlsLocked} className="rounded-xl border border-[#d2e3fc] bg-[#f8faff] p-4">
          <legend className="px-1 text-sm font-semibold text-[#0b57d0]">行动判定（高级项）</legend>
          <label className="mt-2 flex items-start gap-2 text-sm text-[#3c4043]">
            <input
              type="checkbox"
              checked={resolutionMode === 'advanced'}
              onChange={event => setResolutionMode(event.target.checked ? 'advanced' : 'narrative')}
            />
            <span>
              启用小说专属 D20 属性与检定
              <span className="mt-1 block text-xs text-[#5f6368]">默认仍是纯叙事模式；规则模板由同一本小说的玩家共享。</span>
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
                  {generateRules.isPending ? '正在生成小说规则…' : '生成小说专属规则'}
                </button>
              ) : (
                <>
                  <div className="flex items-center justify-between text-xs text-[#5f6368]">
                    <span>属性点 {assignedPoints} / {gameRules.point_budget}</span>
                    <span>D20 · 服务器判定</span>
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
                  {getApiErrorMessage(generateRules.error, '小说规则生成失败，请稍后重试')}
                </p>
              ) : null}
              {gameRules && !advancedReady ? (
                <p role="alert" className="text-sm text-[#b3261e]">
                  属性必须为 {gameRules.minimum_score}–{gameRules.maximum_score} 的整数，且恰好分配 {gameRules.point_budget} 点。
                </p>
              ) : null}
            </div>
          ) : null}
        </fieldset>
        <label className="block text-sm font-medium text-[#3c4043]">
          名字
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
          背景
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
          能力（用逗号分隔）
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
          初始地点
          <select
            className="field-control mt-1"
            value={locationId}
            disabled={controlsLocked}
            onChange={event => setLocationId(event.target.value)}
            required
          >
            {locations.map(location => (
              <option key={location.id} value={location.id}>{location.name}</option>
            ))}
          </select>
        </label>
        <label className="block text-sm font-medium text-[#3c4043]">
          随身物品（可选，用逗号分隔）
          <input
            className="field-control mt-1"
            value={inventory}
            disabled={controlsLocked}
            onChange={event => setInventory(event.target.value)}
            maxLength={6400}
          />
        </label>
        {locations.length === 0 ? <p role="alert" className="text-sm text-[#b3261e]">当前进度没有可用地点。</p> : null}
        {error ? <p role="alert" className="text-sm text-[#b3261e]">{error}</p> : null}
        <button
          type="submit"
          disabled={controlsLocked || locations.length === 0 || !advancedReady}
          className="primary-action"
        >
          {isPending ? '正在进入世界…' : '进入故事'}
        </button>
      </form>
    </section>
  );
}
