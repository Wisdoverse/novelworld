import { Link } from 'react-router-dom';
import { isPredominantlyChinese } from '@/features/chapter-translation';
import { translate as t } from '@/shared/lib/i18n';
import type { SourceRelationshipGraph } from '@/entities/novel';

interface SourceRelationshipsProps {
  novelId: string;
  graph?: SourceRelationshipGraph;
  isLoading: boolean;
  isError: boolean;
  onRetry: () => void;
}

export function SourceRelationships({
  novelId,
  graph,
  isLoading,
  isError,
  onRetry,
}: SourceRelationshipsProps) {
  const names = new Map(graph?.characters.map(character => [character.id, character.name]) ?? []);

  return (
    <section className="surface-card mt-6 p-5 sm:p-6" aria-labelledby="source-relationships-heading">
      <h2 id="source-relationships-heading" className="text-lg font-semibold text-[#203a35]">
        {t('Source relationships')}
      </h2>
      <p className="mt-2 text-sm leading-6 text-[#59645f]">
        {t('These facts come from the original text at your saved reading point; they do not show changes in your world.')}
      </p>

      {isLoading ? (
        <p className="mt-4 text-sm text-[#59645f]" role="status">{t('Loading source relationships…')}</p>
      ) : isError ? (
        <div className="mt-4 flex flex-wrap items-center gap-3" role="alert">
          <p className="text-sm text-[#b3261e]">{t('Source relationships could not be loaded.')}</p>
          <button type="button" className="text-sm font-medium text-[#0b57d0] underline" onClick={onRetry}>
            {t('Retry')}
          </button>
        </div>
      ) : !graph?.relationships.length ? (
        <p className="mt-4 text-sm text-[#59645f]">{t('No cited relationships are available at this reading point.')}</p>
      ) : (
        <ul className="mt-4 divide-y divide-[#e8e3d9]">
          {graph.relationships.map((relationship) => (
            <li key={relationship.id} className="py-5 first:pt-2 last:pb-1">
              <h3 className="font-medium text-[#203a35]">
                <span lang={isPredominantlyChinese(names.get(relationship.from_character_id) ?? '') ? 'zh-CN' : 'en'}>
                  {names.get(relationship.from_character_id)}
                </span>
                <span aria-hidden="true" className="px-2 text-[#718079]">→</span>
                <span className="sr-only">{t('to')}</span>
                <span lang={isPredominantlyChinese(names.get(relationship.to_character_id) ?? '') ? 'zh-CN' : 'en'}>
                  {names.get(relationship.to_character_id)}
                </span>
              </h3>
              <p lang={isPredominantlyChinese(relationship.kind) ? 'zh-CN' : 'en'} className="mt-1 text-xs font-medium uppercase tracking-wide text-[#687971]">
                {relationship.kind}
              </p>
              {relationship.description ? (
                <p lang={isPredominantlyChinese(relationship.description) ? 'zh-CN' : 'en'} className="mt-2 whitespace-pre-wrap text-sm leading-6 text-[#3c4943]">
                  {relationship.description}
                </p>
              ) : null}
              {relationship.source_citations.length ? (
                <ul className="mt-3 space-y-3 border-l-2 border-[#d6e3dc] pl-4">
                  {relationship.source_citations.map((citation, index) => (
                    <li key={`${citation.chapter_number}:${index}`}>
                      <blockquote lang={isPredominantlyChinese(citation.excerpt) ? 'zh-CN' : 'en'} className="text-sm leading-6 text-[#59645f]">
                        {citation.excerpt}
                      </blockquote>
                      <Link
                        className="mt-1 inline-block text-sm font-medium text-[#0b57d0] underline underline-offset-2"
                        to={`/reader/${novelId}/${citation.chapter_number}`}
                      >
                        {t('Chapter {p0}', { p0: citation.chapter_number })}
                      </Link>
                    </li>
                  ))}
                </ul>
              ) : null}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
