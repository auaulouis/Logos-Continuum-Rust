/* eslint-disable react/no-danger */
/* eslint-disable no-nested-ternary */
import {
  useState, useRef, useEffect, useMemo,
} from 'react';
import type { SearchResult, Card } from '../../lib/types';
import type { CardPreference } from '../../lib/cardPreferences';
import { generateStyledCite } from '../../lib/utils';
import DownloadLink from '../DownloadLink';
import styles from './styles.module.scss';

const extractCardIdentifier = (result: SearchResult): string => {
  if (result.card_identifier && result.card_identifier.trim()) {
    return result.card_identifier.trim();
  }

  const tagText = String(result.tag || '');
  const tokenMatch = tagText.match(/\[\[(CID-[^\]]+)\]\]/i);
  if (tokenMatch?.[1]) {
    return tokenMatch[1].trim();
  }

  return '';
};

const stripIdentifierTokenFromTag = (tag: string): string => {
  return String(tag || '').replace(/\s*\[\[CID-[^\]]+\]\]\s*/gi, ' ').trim();
};

const extractTagSubHeadline = (value: string | undefined): string => {
  const lines = String(value || '')
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean);
  return lines[0] || '';
};

const RESULTS_PER_PAGE = 30;
type SearchTab = 'tag' | 'paragraph';

const getVisiblePageIndexes = (currentPage: number, hasMoreResults: boolean): number[] => {
  const pages: number[] = [];

  if (currentPage > 0) {
    pages.push(currentPage - 1);
  }

  pages.push(currentPage);

  if (hasMoreResults) {
    pages.push(currentPage + 1);
  }

  return pages;
};

type SearchResultsProps = {
  tabResults: Record<SearchTab, Array<SearchResult>>;
  tabCounts: Record<SearchTab, number>;
  tabCountsPartial: Record<SearchTab, boolean>;
  searchDurationsMs: Record<SearchTab, number>;
  query: string;
  setSelected: (id: string) => void;
  cards: Record<string, any>;
  getCard: (id: string) => Promise<Card | undefined>;
  loadPage: (tab: SearchTab, page: number) => Promise<boolean>;
  setDownloadUrls: (urls: string[]) => void;
  tabHasMoreResults: Record<SearchTab, boolean>;
  cardPreferences: Record<string, CardPreference>;
  activeTab: SearchTab;
  onTabChange: (tab: SearchTab) => void;
  loading: boolean;
};

const SearchResults = ({
  tabResults,
  tabCounts,
  tabCountsPartial,
  searchDurationsMs,
  query,
  setSelected,
  cards,
  getCard,
  loadPage,
  setDownloadUrls,
  tabHasMoreResults,
  cardPreferences,
  activeTab,
  onTabChange,
  loading,
}: SearchResultsProps) => {
  const [loadingMore, setLoadingMore] = useState(false);
  const [tabPages, setTabPages] = useState<Record<SearchTab, number>>({ tag: 0, paragraph: 0 });
  const [pendingNextPage, setPendingNextPage] = useState<number | null>(null);
  const resultsContainer = useRef<HTMLDivElement>(null);
  const activeResults = tabResults[activeTab] || [];
  const hasMoreResults = !!tabHasMoreResults[activeTab];
  const currentPage = tabPages[activeTab];
  const pageResults = activeResults.slice(0, RESULTS_PER_PAGE);
  const pageIndexes = useMemo(
    () => getVisiblePageIndexes(currentPage, hasMoreResults),
    [currentPage, hasMoreResults],
  );
  const activeSearchSeconds = (searchDurationsMs[activeTab] || 0) / 1000;
  const formattedSearchTime = `${activeSearchSeconds.toFixed(2)} s`;

  useEffect(() => {
    setTabPages({ tag: 0, paragraph: 0 });
    setPendingNextPage(null);
  }, [query]);

  useEffect(() => {
    if (pendingNextPage === null) {
      return;
    }

    if (loadingMore || loading) {
      return;
    }

    const targetPage = pendingNextPage;
    setLoadingMore(true);
    loadPage(activeTab, targetPage).then((didLoad) => {
      if (didLoad) {
        setTabPages((prev) => ({ ...prev, [activeTab]: targetPage }));
      }
    }).finally(() => {
      setLoadingMore(false);
      setPendingNextPage(null);
    });
  }, [pendingNextPage, activeTab, loadingMore, loading]);

  useEffect(() => {
    if (resultsContainer.current) {
      resultsContainer.current.scrollTop = 0;
    }
  }, [activeTab, currentPage]);

  const handleTabClick = (tab: SearchTab) => {
    onTabChange(tab);
    setPendingNextPage(null);
  };

  const onPrevPage = () => {
    const previousPage = Math.max(0, currentPage - 1);
    if (previousPage === currentPage || loadingMore) {
      return;
    }
    setPendingNextPage(previousPage);
  };

  const onNextPage = async () => {
    const nextPage = currentPage + 1;
    if (!hasMoreResults || loadingMore) {
      return;
    }

    setPendingNextPage(nextPage);
  };

  const onPageSelect = async (targetPage: number) => {
    if (targetPage < 0 || targetPage === currentPage) {
      return;
    }

    if (loadingMore) {
      return;
    }

    if (targetPage > currentPage && !hasMoreResults) {
      return;
    }

    setPendingNextPage(targetPage);
  };

  const renderResult = (result: SearchResult, index: number) => {
    const card = cards[result.id];
    const cardIdentifier = extractCardIdentifier(result);
    const displayTag = stripIdentifierTokenFromTag(result.tag)
      || extractTagSubHeadline(result.tag_sub)
      || cardIdentifier;
    const preference = cardPreferences[result.id];

    const onClick = () => {
      setSelected(result.id);
      if (result.download_url) {
        setDownloadUrls(Array.isArray(result.download_url) ? result.download_url : [result.download_url]);
      }
    };

    return (
      <div key={`${result.id}-${index}`} className={styles.result} role="button" tabIndex={0} onClick={onClick}>
        <div className={styles['result-header']}>
          <div className={styles.tag}>{/\d/.test(result.cite) ? displayTag : `${displayTag} ${result.cite}`}</div>
          <div className={styles['result-meta']}>
            {Boolean(preference?.starred) && <div className={styles.cid}>★</div>}
            {!!preference?.customTag && <div className={styles.cid}>#{preference.customTag}</div>}
            {cardIdentifier && <div className={styles.cid}>{cardIdentifier}</div>}
          </div>
        </div>
        <div className={styles.cite}
          dangerouslySetInnerHTML={{
            __html: (/\d/.test(result.cite)
              ? generateStyledCite(result.cite, result.cite_emphasis, 11)
              : (card ? card.body.find((p: string) => /\d/.test(p)) : result.cite)),
          }}
        />
        <DownloadLink url={result.download_url} />
      </div>
    );
  };

  const formatTabCount = (count: number, isPartial?: boolean) => {
    if (count < 0) return '...';
    return isPartial ? `${count}+` : count.toLocaleString();
  };

  return (
    <div className={styles['results-shell']}>
      <div className={styles['results-tabs']}>
        <div className={styles['results-tabs-left']}>
          <button
            type="button"
            className={`${styles['results-tab']} ${activeTab === 'tag' ? styles['results-tab-active'] : ''}`}
            onClick={() => handleTabClick('tag')}
            disabled={loading}
          >
            Tag Matches ({formatTabCount(tabCounts.tag, tabCountsPartial.tag)})
          </button>
          <button
            type="button"
            className={`${styles['results-tab']} ${activeTab === 'paragraph' ? styles['results-tab-active'] : ''}`}
            onClick={() => handleTabClick('paragraph')}
            disabled={loading}
          >
            Paragraph Matches ({formatTabCount(tabCounts.paragraph, tabCountsPartial.paragraph)})
          </button>
        </div>
        <div className={styles['results-tabs-center']}>
          {formattedSearchTime}
        </div>
        <div className={styles['results-tabs-right']}>
          <div className={styles['results-tabs-pages']}>
            {pageIndexes.map((pageIndex) => (
              <button
                key={`top-${pageIndex}`}
                type="button"
                className={`${styles['page-number']} ${pageIndex === currentPage ? styles['page-number-active'] : ''}`}
                onClick={() => { onPageSelect(pageIndex); }}
                disabled={loadingMore}
              >
                {pageIndex + 1}
              </button>
            ))}
          </div>
          <button
            type="button"
            className={styles['page-button']}
            onClick={onPrevPage}
            disabled={currentPage === 0 || loadingMore}
          >
            Previous
          </button>
          <button
            type="button"
            className={styles['page-button']}
            onClick={() => { onNextPage(); }}
            disabled={loadingMore || !hasMoreResults}
          >
            Next
          </button>
        </div>
      </div>

      <div className={styles.results} ref={resultsContainer}>

        {pageResults.map(renderResult)}

        {pageResults.length === 0 && (
          <div className={styles['end-of-results']}>
            {activeTab === 'tag' ? 'No tag matches in loaded results yet' : 'No paragraph matches in loaded results yet'}
          </div>
        )}

        {(activeResults.length > 0 || hasMoreResults) && (
          <div className={styles.pagination}>
            <button
              type="button"
              className={styles['page-button']}
              onClick={onPrevPage}
              disabled={currentPage === 0 || loadingMore}
            >
              Previous
            </button>
            <div className={styles['page-numbers']}>
              {pageIndexes.map((pageIndex) => (
                <button
                  key={pageIndex}
                  type="button"
                  className={`${styles['page-number']} ${pageIndex === currentPage ? styles['page-number-active'] : ''}`}
                  onClick={() => { onPageSelect(pageIndex); }}
                  disabled={loadingMore}
                >
                  {pageIndex + 1}
                </button>
              ))}
            </div>
            <button
              type="button"
              className={styles['page-button']}
              onClick={() => { onNextPage(); }}
              disabled={loadingMore || !hasMoreResults}
            >
              Next
            </button>
          </div>
        )}

        {!hasMoreResults && activeResults.length > 0 && (
          <div className={styles['end-of-results']}>End of results</div>
        )}
      </div>
    </div>
  );
};

export default SearchResults;
