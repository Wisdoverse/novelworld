import { translate as t } from '@/shared/lib/i18n';
import { isAxiosError } from 'axios';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { apiClient } from '@/shared/api/client';
import { storePendingWorldSource } from '@/shared/lib/worldSourceStorage';
import { removeWorldTurnPendingRequest } from '@/shared/lib/worldTurnStorage';
import type {
  Novel,
  Chapter,
  Character,
  WorldSeries,
  WorldSeriesBackgroundDraft,
  SeriesBackgroundDraft,
  WorldSeriesSuggestion,
} from '@/shared/types';

// ─── Query Keys ───────────────────────────────────────────────────────────────
export const novelKeys = {
  all: ['novels'] as const,
  list: () => [...novelKeys.all, 'list'] as const,
  catalog: () => [...novelKeys.all, 'catalog'] as const,
  detail: (id: string) => [...novelKeys.all, 'detail', id] as const,
  chapter: (id: string, num: number) => [...novelKeys.all, id, 'chapters', num] as const,
  characters: (id: string, chapter: number) => [...novelKeys.all, id, 'characters', chapter] as const,
  sourceRelationships: (principalId: string, id: string, chapter: number) => [
    ...novelKeys.all, 'source-relationships', principalId, id, chapter,
  ] as const,
  worldSeriesList: (principalId: string) => [...novelKeys.all, 'world-series', principalId, 'list'] as const,
  novelWorldSeries: (principalId: string, novelId: string) => [
    ...novelKeys.all, 'world-series', principalId, 'novel', novelId,
  ] as const,
  worldSeriesContribution: (principalId: string, seriesId: string) => [
    ...novelKeys.all, 'world-series', principalId, 'contribution', seriesId,
  ] as const,
};

export interface CreateWorldSeriesInput {
  name: string;
  background: string | null;
  source_novel_id: string;
  canon_model_version?: number;
}

// ─── Hooks ────────────────────────────────────────────────────────────────────

export function shouldPollNovelList(novels: Novel[] | undefined) {
  return novels?.some(
    novel => novel.status === 'pending' || novel.status === 'parsing',
  ) ?? false;
}

export function useNovels() {
  return useQuery({
    queryKey: novelKeys.list(),
    queryFn: () => apiClient.get<Novel[]>('/novels').then(r => r.data),
    refetchInterval: (query) => shouldPollNovelList(query.state.data) ? 2000 : false,
  });
}

export function useNovelCatalog() {
  return useQuery({
    queryKey: novelKeys.catalog(),
    queryFn: () => apiClient.get<Novel[]>('/novels/catalog').then(r => r.data),
  });
}

export function useWorldSeriesList(principalId: string | undefined) {
  return useQuery({
    queryKey: novelKeys.worldSeriesList(principalId ?? ''),
    queryFn: ({ signal }) => apiClient
      .get<WorldSeries[]>('/novels/world-series', { signal })
      .then(response => response.data),
    enabled: Boolean(principalId),
  });
}

export function useNovelWorldSeries(principalId: string | undefined, novelId: string) {
  return useQuery({
    queryKey: novelKeys.novelWorldSeries(principalId ?? '', novelId),
    queryFn: ({ signal }) => apiClient
      .get<WorldSeries | null>(`/novels/${novelId}/world-series`, { signal })
      .then(response => response.data),
    enabled: Boolean(principalId && novelId),
  });
}

export function useWorldSeriesBackgroundDraft() {
  return useMutation({
    retry: false,
    mutationFn: (novelId: string) => apiClient
      .get<WorldSeriesBackgroundDraft>(`/novels/${novelId}/world-series/background-draft`)
      .then(response => response.data),
  });
}

export function useWorldSeriesContribution(principalId: string | undefined, seriesId: string | undefined) {
  return useQuery({
    queryKey: novelKeys.worldSeriesContribution(principalId ?? '', seriesId ?? ''),
    queryFn: ({ signal }) => apiClient
      .get<{ enabled: boolean }>(`/novels/world-series/${seriesId}/contribution`, { signal })
      .then(response => response.data),
    enabled: Boolean(principalId && seriesId),
    retry: false,
  });
}

export function useSetWorldSeriesContribution(principalId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    retry: false,
    mutationFn: ({ seriesId, enabled }: { seriesId: string; enabled: boolean }) => apiClient
      .put<{ enabled: boolean }>(`/novels/world-series/${seriesId}/contribution`, { enabled })
      .then(response => response.data),
    onSuccess: async (_data, { seriesId }) => {
      await queryClient.invalidateQueries({ queryKey: novelKeys.worldSeriesContribution(principalId, seriesId) });
    },
  });
}

export function useCommunitySeriesSuggestion() {
  return useMutation({
    retry: false,
    mutationFn: (novelId: string) => apiClient
      .post<WorldSeriesSuggestion>(`/novels/${novelId}/world-series/community-suggestion`)
      .then(response => response.data),
  });
}

export function useSeriesBackgroundDraft() {
  return useMutation({
    retry: false,
    mutationFn: (seriesId: string) => apiClient
      .get<SeriesBackgroundDraft>(`/novels/world-series/${seriesId}/background-draft`)
      .then(response => response.data),
  });
}

export function useSuggestNovelWorldSeries() {
  return useMutation({
    retry: false,
    mutationFn: (novelId: string) => apiClient
      .post<WorldSeriesSuggestion>(`/novels/${novelId}/world-series/suggestion`)
      .then(response => response.data),
  });
}

export function useSuggestNovelWorldSeriesDeepSeek() {
  return useMutation({
    retry: false,
    mutationFn: ({ novelId, checkOnly = false }: { novelId: string; checkOnly?: boolean }) => apiClient
      .post<WorldSeriesSuggestion>(
        `/novels/${novelId}/world-series/suggestion/deepseek`,
        undefined,
        {
          timeout: 60_000,
          ...(checkOnly ? { params: { check_only: true } } : {}),
        },
      )
      .then(response => response.data),
  });
}

export function useCreateWorldSeries(principalId: string | undefined) {
  const queryClient = useQueryClient();
  return useMutation({
    retry: false,
    mutationFn: (input: CreateWorldSeriesInput) => apiClient
      .post<WorldSeries>('/novels/world-series', input)
      .then(response => response.data),
    onSuccess: (_series, input) => {
      if (!principalId) return;
      queryClient.invalidateQueries({ queryKey: novelKeys.worldSeriesList(principalId) });
      queryClient.invalidateQueries({
        queryKey: novelKeys.novelWorldSeries(principalId, input.source_novel_id),
      });
    },
  });
}

export function useConfirmWorldSeriesBackground(principalId: string | undefined) {
  const queryClient = useQueryClient();
  return useMutation({
    retry: false,
    mutationFn: ({ seriesId, background }: { seriesId: string; background: string }) => apiClient
      .put<WorldSeries>(`/novels/world-series/${seriesId}/background`, { background })
      .then(response => response.data),
    onSuccess: () => {
      if (!principalId) return;
      queryClient.invalidateQueries({ queryKey: novelKeys.worldSeriesList(principalId) });
      queryClient.invalidateQueries({ queryKey: [...novelKeys.all, 'world-series', principalId, 'novel'] });
    },
  });
}

export function useAssociateNovelWorldSeries(principalId: string | undefined, novelId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    retry: false,
    mutationFn: (seriesId: string | null) => apiClient
      .put<WorldSeries | null>(`/novels/${novelId}/world-series`, { series_id: seriesId })
      .then(response => response.data),
    onSuccess: () => {
      if (!principalId) return;
      queryClient.invalidateQueries({ queryKey: novelKeys.novelWorldSeries(principalId, novelId) });
      queryClient.invalidateQueries({ queryKey: novelKeys.worldSeriesList(principalId) });
    },
  });
}

export function useNovel(id: string) {
  return useQuery({
    queryKey: novelKeys.detail(id),
    queryFn: () => apiClient.get<Novel>(`/novels/${id}`).then(r => r.data),
    enabled: !!id,
  });
}

export function useChapter(novelId: string, chapterNum: number) {
  return useQuery({
    queryKey: novelKeys.chapter(novelId, chapterNum),
    queryFn: () => apiClient.get<Chapter>(`/novels/${novelId}/chapters/${chapterNum}`).then(r => r.data),
    enabled: !!novelId && chapterNum > 0,
  });
}

const PARTIAL_CHARACTER_KEYS = new Set([
  'id',
  'novel_id',
  'name',
  'first_appearance_chapter',
]);

export function sanitizeCharacterPersona(
  character: Character,
  currentChapter: number,
): Character | null {
  if (!Number.isSafeInteger(currentChapter) || currentChapter < 1) return null;

  const firstAppearance = character.first_appearance_chapter;
  if (
    typeof firstAppearance !== 'number'
    || !Number.isSafeInteger(firstAppearance)
    || firstAppearance < 1
    || firstAppearance > currentChapter
  ) return null;

  const highWater = character.persona_source_chapter_high_water;
  if (
    typeof highWater === 'number'
    && Number.isSafeInteger(highWater)
    && highWater >= 1
    && highWater <= currentChapter
  ) {
    return {
      id: character.id,
      novel_id: character.novel_id,
      name: character.name,
      aliases: character.aliases,
      role: character.role,
      description: character.description,
      personality: character.personality,
      background: character.background,
      speaking_style: character.speaking_style,
      appearance: character.appearance,
      avatar_url: character.avatar_url,
      avatar_status: character.avatar_status,
      first_appearance_chapter: firstAppearance,
      persona_source_chapter_high_water: highWater,
    };
  }

  const keys = Object.keys(character);
  if (
    keys.length !== PARTIAL_CHARACTER_KEYS.size
    || keys.some(key => !PARTIAL_CHARACTER_KEYS.has(key))
  ) return null;

  return {
    id: character.id,
    novel_id: character.novel_id,
    name: character.name,
    first_appearance_chapter: firstAppearance,
  };
}

export function useCharacters(novelId: string, currentChapter: number, enabled = true) {
  return useQuery({
    queryKey: novelKeys.characters(novelId, currentChapter),
    queryFn: () => apiClient
      .get<Character[]>(`/novels/${novelId}/characters`)
      .then(r => r.data
        .map(character => sanitizeCharacterPersona(character, currentChapter))
        .filter((character): character is Character => character !== null)),
    enabled: enabled && !!novelId && currentChapter >= 1,
  });
}

export interface SourceRelationshipCitation {
  chapter_number: number;
  excerpt: string;
}

export interface SourceRelationshipCharacter {
  id: string;
  name: string;
}

export interface SourceRelationship {
  id: string;
  from_character_id: string;
  to_character_id: string;
  kind: string;
  description: string;
  source_citations: SourceRelationshipCitation[];
}

export interface SourceRelationshipGraph {
  novel_id: string;
  model_version: 1;
  checkpoint_chapter: number;
  characters: SourceRelationshipCharacter[];
  relationships: SourceRelationship[];
}

const SOURCE_GRAPH_UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const SOURCE_GRAPH_NIL_UUID = '00000000-0000-0000-0000-000000000000';

function sourceGraphUuid(value: unknown): value is string {
  return typeof value === 'string'
    && SOURCE_GRAPH_UUID.test(value)
    && value.toLowerCase() !== SOURCE_GRAPH_NIL_UUID;
}

function exactObjectKeys(value: unknown, expected: readonly string[]): value is Record<string, unknown> {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return false;
  const keys = Object.keys(value);
  return keys.length === expected.length && keys.every(key => expected.includes(key));
}

function boundedSourceText(value: unknown, maxCharacters: number): value is string {
  return typeof value === 'string'
    && value.trim().length > 0
    && Array.from(value).length <= maxCharacters
    && !/(?![\n\r\t])\p{Cc}/u.test(value);
}

function sourceChapter(value: unknown, currentChapter: number): value is number {
  return typeof value === 'number'
    && Number.isSafeInteger(value)
    && value >= 1
    && value <= currentChapter;
}

/** Accept only the source-v1 fields and evidence visible at the trusted progress boundary. */
export function parseSourceRelationshipGraph(
  value: unknown,
  novelId: string,
  currentChapter: number,
): SourceRelationshipGraph | null {
  if (
    !sourceGraphUuid(novelId)
    || !Number.isSafeInteger(currentChapter)
    || currentChapter < 1
    || !exactObjectKeys(value, [
      'novel_id', 'model_version', 'checkpoint_chapter', 'characters', 'relationships',
    ])
    || value.novel_id !== novelId
    || value.model_version !== 1
    || !sourceChapter(value.checkpoint_chapter, currentChapter)
    || value.checkpoint_chapter !== currentChapter
    || !Array.isArray(value.characters)
    || value.characters.length > 256
    || !Array.isArray(value.relationships)
    || value.relationships.length > 256
  ) return null;

  const characterNames = new Map<string, string>();
  for (const character of value.characters) {
    if (
      !exactObjectKeys(character, ['id', 'name'])
      || !sourceGraphUuid(character.id)
      || !boundedSourceText(character.name, 200)
      || characterNames.has(character.id)
    ) return null;
    characterNames.set(character.id, character.name);
  }

  const relationshipIds = new Set<string>();
  const relationships: SourceRelationship[] = [];
  for (const relationship of value.relationships) {
    if (
      !exactObjectKeys(relationship, [
        'id', 'from_character_id', 'to_character_id', 'kind', 'description', 'source_citations',
      ])
      || !boundedSourceText(relationship.id, 100)
      || !sourceGraphUuid(relationship.from_character_id)
      || !characterNames.has(relationship.from_character_id)
      || !sourceGraphUuid(relationship.to_character_id)
      || relationship.from_character_id === relationship.to_character_id
      || !characterNames.has(relationship.to_character_id)
      || !boundedSourceText(relationship.kind, 500)
      || !boundedSourceText(relationship.description, 10_000)
      || !Array.isArray(relationship.source_citations)
      || relationship.source_citations.length === 0
      || relationship.source_citations.length > 8
      || relationshipIds.has(relationship.id)
    ) return null;

    const citations: SourceRelationshipCitation[] = [];
    for (const citation of relationship.source_citations) {
      if (
        !exactObjectKeys(citation, ['chapter_number', 'excerpt'])
        || !sourceChapter(citation.chapter_number, value.checkpoint_chapter)
        || !boundedSourceText(citation.excerpt, 2_000)
      ) return null;
      citations.push({ chapter_number: citation.chapter_number, excerpt: citation.excerpt });
    }

    relationshipIds.add(relationship.id);
    relationships.push({
      id: relationship.id,
      from_character_id: relationship.from_character_id,
      to_character_id: relationship.to_character_id,
      kind: relationship.kind,
      description: relationship.description,
      source_citations: citations,
    });
  }

  return {
    novel_id: value.novel_id,
    model_version: 1,
    checkpoint_chapter: value.checkpoint_chapter,
    characters: value.characters.map(character => ({ id: character.id as string, name: character.name as string })),
    relationships,
  };
}

export function useSourceRelationships(
  principalId: string | undefined,
  novelId: string,
  currentChapter: number,
  enabled = true,
) {
  return useQuery({
    queryKey: novelKeys.sourceRelationships(principalId ?? '', novelId, currentChapter),
    retry: false,
    queryFn: async ({ signal }) => {
      const response = await apiClient.get<unknown>(
        `/novels/${novelId}/relationships/source-v1`,
        { signal },
      );
      const graph = parseSourceRelationshipGraph(response.data, novelId, currentChapter);
      if (!graph) throw new Error('Invalid source relationship response');
      return graph;
    },
    enabled: Boolean(
      enabled && principalId && novelId && Number.isSafeInteger(currentChapter) && currentChapter >= 1,
    ),
  });
}

export function useImportNovel() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (data: {
      title: string;
      author?: string;
      content?: string;
      deviation_mode?: string;
    }) => apiClient.post<{ novel_id: string; status: string }>('/novels', data).then(r => r.data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: novelKeys.list() });
    },
  });
}

export interface NovelUploadInput {
  title: string;
  author?: string;
  deviationMode: string;
  file: File;
}

export interface NovelBatchUploadInput {
  author?: string;
  deviationMode: string;
  files: File[];
}

export interface NovelImportAccepted {
  novel_id: string;
  status: string;
}

export const MAX_NOVEL_BATCH_FILES = 5;
export const MAX_NOVEL_UPLOAD_FILES = 50;
export const MAX_NOVEL_BATCH_BYTES = 40 * 1024 * 1024;

export function buildNovelUploadFormData(input: NovelUploadInput) {
  const form = new FormData();
  form.append('title', input.title);
  if (input.author) form.append('author', input.author);
  form.append('deviation_mode', input.deviationMode);
  form.append('file', input.file);
  return form;
}

export function buildNovelBatchUploadFormData(input: NovelBatchUploadInput) {
  const form = new FormData();
  if (input.author) form.append('author', input.author);
  form.append('deviation_mode', input.deviationMode);
  input.files.forEach(file => form.append('file', file));
  return form;
}

export function novelTitleFromFile(file: File) {
  return file.name.replace(/\.(txt|epub|pdf)$/i, '');
}

export function validateNovelFile(file: File): string | null {
  const extension = file.name.split('.').pop()?.toLowerCase();
  if (!extension || !['txt', 'epub', 'pdf'].includes(extension)) {
    return t("Choose a TXT, EPUB or PDF file");
  }
  const limit = extension === 'txt' ? 10 * 1024 * 1024 : 20 * 1024 * 1024;
  if (file.size > limit) {
    return t("{p0} files cannot exceed {p1} MiB", { p0: extension.toUpperCase(), p1: limit / 1024 / 1024 });
  }
  return null;
}

export function validateNovelBatchFiles(files: File[]): string | null {
  if (!files.length) return t("Choose at least one novel");
  if (files.length > MAX_NOVEL_UPLOAD_FILES) {
    return t("Import at most {p0} novels at a time", { p0: MAX_NOVEL_UPLOAD_FILES });
  }
  for (const file of files) {
    const error = validateNovelFile(file);
    if (error) return `${file.name}：${error}`;
  }
  return null;
}

export function splitNovelUploadBatches(files: File[]): File[][] {
  const validation = validateNovelBatchFiles(files);
  if (validation) throw new Error(validation);
  const batches: File[][] = [];
  let bytes = 0;
  for (const file of files) {
    let batch = batches[batches.length - 1];
    if (!batch || batch.length === MAX_NOVEL_BATCH_FILES || bytes + file.size > MAX_NOVEL_BATCH_BYTES) {
      batch = [];
      batches.push(batch);
      bytes = 0;
    }
    batch.push(file);
    bytes += file.size;
  }
  return batches;
}

export class NovelBatchUploadError extends Error {
  constructor(
    readonly accepted: NovelImportAccepted[],
    readonly unknownFiles: File[],
    readonly remainingFiles: File[],
    readonly cause: unknown,
    readonly reason: 'request_failed' | 'session_changed' = 'request_failed',
  ) {
    super('Novel upload stopped before all files were confirmed');
  }
}

export function useUploadNovel() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: NovelUploadInput) => apiClient.post<{
      novel_id: string;
      status: string;
    }>('/novels/upload', buildNovelUploadFormData(input), {
      timeout: 60_000,
    }).then(r => r.data),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: novelKeys.list() }),
  });
}

export function useUploadNovelsBatch() {
  const queryClient = useQueryClient();
  return useMutation({
    retry: false,
    mutationFn: async (input: NovelBatchUploadInput) => {
      const batches = splitNovelUploadBatches(input.files);
      const accepted: NovelImportAccepted[] = [];
      const accessToken = localStorage.getItem('auth_token');
      const headers = { Authorization: accessToken ? `Bearer ${accessToken}` : '' };
      for (let index = 0; index < batches.length; index++) {
        const files = batches[index];
        if (localStorage.getItem('auth_token') !== accessToken) {
          throw new NovelBatchUploadError(accepted, [], batches.slice(index).flat(),
            new Error('Upload session changed'), 'session_changed');
        }
        try {
          const response = await apiClient.post<{ novels: NovelImportAccepted[] }>(
            '/novels/upload/batch', buildNovelBatchUploadFormData({ ...input, files }),
            { timeout: 120_000, headers },
          );
          if (!Array.isArray(response.data.novels) || response.data.novels.length !== files.length
            || response.data.novels.some(novel => !novel.novel_id)) {
            throw new Error('Invalid import acceptance response');
          }
          accepted.push(...response.data.novels);
          void queryClient.invalidateQueries({ queryKey: novelKeys.list() });
        } catch (cause) {
          const status = isAxiosError(cause) ? cause.response?.status : undefined;
          const unknown = status === undefined || status >= 500;
          throw new NovelBatchUploadError(
            accepted, unknown ? files : [],
            [...(unknown ? [] : files), ...batches.slice(index + 1).flat()], cause,
          );
        }
      }
      return { novels: accepted };
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: novelKeys.list() }),
  });
}

export function useDeleteNovel(userId?: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => apiClient.delete(`/novels/${id}`),
    onSuccess: (_response, novelId) => {
      if (userId) {
        removeWorldTurnPendingRequest(userId, novelId);
        storePendingWorldSource(userId, novelId, null);
      }
      queryClient.invalidateQueries({ queryKey: novelKeys.list() });
      queryClient.invalidateQueries({ queryKey: novelKeys.catalog() });
    },
  });
}

export function useAttachNovel() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ novelId, deviationMode }: { novelId: string; deviationMode: string }) =>
      apiClient.post(`/novels/${novelId}/shelf`, { deviation_mode: deviationMode }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: novelKeys.list() });
      queryClient.invalidateQueries({ queryKey: novelKeys.catalog() });
    },
  });
}

export function useRetryNovel() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => apiClient.post(`/novels/${id}/retry`),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: novelKeys.list() }),
  });
}
