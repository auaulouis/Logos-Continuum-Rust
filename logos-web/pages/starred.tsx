/* eslint-disable jsx-a11y/anchor-is-valid */
import {
  useCallback, useEffect, useMemo, useState,
} from 'react';
import Head from 'next/head';
import Link from 'next/link';
import { useRouter } from 'next/router';
import pageStyles from '../styles/index.module.scss';
import queryStyles from '../components/query/styles.module.scss';
import CardDetail from '../components/query/CardDetail';
import { Card } from '../lib/types';
import * as apiService from '../services/api';
import { applySavedEdit } from '../lib/cardEdits';
import {
  CardPreference,
  getAllCardPreferences,
  getAllCustomTags,
  setCardCustomTag,
  setCardStarred,
  subscribeToCardPreferences,
  updateCardPreferenceSnapshot,
} from '../lib/cardPreferences';

const StarredPage = () => {
  const router = useRouter();
  const [preferences, setPreferences] = useState<Record<string, CardPreference>>({});
  const [cards, setCards] = useState<Record<string, Card>>({});
  const [selectedCard, setSelectedCard] = useState('');
  const [selectedTagFilter, setSelectedTagFilter] = useState('all');
  const [tagSearchDraft, setTagSearchDraft] = useState('');
  const [tagSearchQuery, setTagSearchQuery] = useState('');
  const [downloadUrls, setDownloadUrls] = useState<string[]>([]);
  const [tagDraft, setTagDraft] = useState('');
  const [isTagPickerOpen, setIsTagPickerOpen] = useState(false);

  const refreshPreferences = useCallback(() => {
    setPreferences(getAllCardPreferences());
  }, []);

  useEffect(() => {
    refreshPreferences();
  }, [refreshPreferences]);

  useEffect(() => {
    const unsubscribe = subscribeToCardPreferences(() => {
      refreshPreferences();
    });

    return unsubscribe;
  }, [refreshPreferences]);

  const allTags = useMemo(() => getAllCustomTags(), [preferences]);

  useEffect(() => {
    if (selectedTagFilter !== 'all' && !allTags.includes(selectedTagFilter)) {
      setSelectedTagFilter('all');
    }
  }, [allTags, selectedTagFilter]);

  const savedIds = useMemo(() => {
    return Object.entries(preferences)
      .filter(([, preference]) => Boolean(preference?.starred) || Boolean(String(preference?.customTag || '').trim()))
      .sort((a, b) => (b[1]?.updatedAt || 0) - (a[1]?.updatedAt || 0))
      .map(([cardId]) => cardId);
  }, [preferences]);

  const filteredSavedIds = useMemo(() => {
    if (selectedTagFilter === 'all') {
      return savedIds;
    }

    return savedIds.filter((cardId) => {
      const tag = String(preferences[cardId]?.customTag || '').trim();
      return tag === selectedTagFilter;
    });
  }, [savedIds, selectedTagFilter, preferences]);

  const searchedSavedIds = useMemo(() => {
    const normalizedQuery = tagSearchQuery.trim().toLowerCase();
    if (!normalizedQuery) {
      return filteredSavedIds;
    }

    return filteredSavedIds.filter((cardId) => {
      const card = cards[cardId];
      const preference = preferences[cardId];
      const snapshot = preference?.snapshot;
      const searchableFields = [
        card?.tag,
        card?.cite,
        card?.card_identifier,
        card?.body?.join(' '),
        snapshot?.tag,
        snapshot?.cite,
        snapshot?.card_identifier,
        preference?.customTag,
      ]
        .map((value) => String(value || '').toLowerCase())
        .filter(Boolean);

      return searchableFields.some((value) => value.includes(normalizedQuery));
    });
  }, [filteredSavedIds, tagSearchQuery, cards, preferences]);

  useEffect(() => {
    if (!searchedSavedIds.length) {
      setSelectedCard('');
      setTagDraft('');
      setIsTagPickerOpen(false);
      return;
    }

    if (!searchedSavedIds.includes(selectedCard)) {
      setSelectedCard(searchedSavedIds[0]);
    }
  }, [searchedSavedIds, selectedCard]);

  const selectedPreference = selectedCard ? preferences[selectedCard] : undefined;
  const selectedCardData = selectedCard ? cards[selectedCard] : undefined;
  const isSelectedCardStarred = Boolean(selectedPreference?.starred);

  useEffect(() => {
    setTagDraft(selectedPreference?.customTag || '');
  }, [selectedCard, selectedPreference?.customTag]);

  const getCard = useCallback(async (id: string): Promise<Card | undefined> => {
    if (cards[id]) {
      return cards[id];
    }

    try {
      const remoteCard = await apiService.getCard(id);
      const hydratedCard = applySavedEdit(remoteCard);
      setCards((previous) => ({ ...previous, [id]: hydratedCard }));
      updateCardPreferenceSnapshot(hydratedCard);
      return hydratedCard;
    } catch {
      return undefined;
    }
  }, [cards]);

  useEffect(() => {
    if (!selectedCard) {
      return;
    }

    (async () => {
      const hydratedCard = await getCard(selectedCard);
      const selectedUrls = hydratedCard?.download_url;
      if (selectedUrls) {
        setDownloadUrls(Array.isArray(selectedUrls) ? selectedUrls : [selectedUrls]);
      } else {
        setDownloadUrls([]);
      }
    })();
  }, [selectedCard, getCard]);

  const toggleSelectedCardStar = async () => {
    if (!selectedCard) return;

    const card = selectedCardData || await getCard(selectedCard);
    if (!card) return;

    setCardStarred(card, !isSelectedCardStarred);
    setIsTagPickerOpen(false);
    refreshPreferences();
  };

  const openSelectedCardInSearch = () => {
    if (!selectedCard) {
      return;
    }

    router.push({
      pathname: '/query',
      query: {
        open_card: selectedCard,
        edit_card: 'true',
      },
    });
  };

  const saveSelectedCardTag = async (nextTagValue?: string) => {
    if (!selectedCard) return;

    const card = selectedCardData || await getCard(selectedCard);
    if (!card) return;

    setCardCustomTag(card, (nextTagValue ?? tagDraft).trim());
    refreshPreferences();
    setIsTagPickerOpen(false);
  };

  const renderStarredCard = (cardId: string) => {
    const hydratedCard = cards[cardId];
    const preference = preferences[cardId];
    const snapshot = preference?.snapshot;
    const cardIdentifier = hydratedCard?.card_identifier || snapshot?.card_identifier || '';

    const tagText = hydratedCard?.tag || snapshot?.tag || cardId;
    const citeText = hydratedCard?.cite || snapshot?.cite || '';

    return (
      <div
        key={cardId}
        className={queryStyles.result}
        role="button"
        tabIndex={0}
        onClick={() => {
          setSelectedCard(cardId);
          const url = hydratedCard?.download_url;
          if (url) {
            setDownloadUrls(Array.isArray(url) ? url : [url]);
          }
        }}
      >
        <div className={queryStyles['result-header']}>
          <div className={queryStyles.tag}>{tagText}</div>
          <div className={queryStyles['result-meta']}>
            {Boolean(preference?.starred) && <div className={queryStyles.cid}>★</div>}
            {!!preference?.customTag && <div className={queryStyles.cid}>#{preference.customTag}</div>}
            {!!cardIdentifier && <div className={queryStyles.cid}>{cardIdentifier}</div>}
          </div>
        </div>
        <div className={queryStyles.cite}>{citeText}</div>
      </div>
    );
  };

  return (
    <>
      <Head>
        <title>Starred Cards - Logos Continuum</title>
        <meta name="description" content="View starred cards and filter by custom tags" />
      </Head>

      <div className={pageStyles.container}>
        <div className={pageStyles.foreground}>
          <div className="query-shell">
            <div className="logo query-logo">
              <Link href="/" passHref><a><h1 className={pageStyles.logo}>Logos Continuum</h1></a></Link>
            </div>

            <div className="query-page">
              <div className={queryStyles['starred-layout']}>
                <div className={queryStyles['starred-search-pane']}>
                  <div className={queryStyles['input-container']}>
                    <div className={queryStyles['query-controls-row']}>
                      <input
                        type="text"
                        className={queryStyles.search}
                        value={tagSearchDraft}
                        onChange={(event) => {
                          setTagSearchDraft(event.currentTarget.value);
                        }}
                        onKeyDown={(event) => {
                          if (event.key === 'Enter') {
                            setTagSearchQuery(tagSearchDraft.trim());
                          }
                        }}
                        placeholder="Search starred cards"
                      />
                      <button
                        type="button"
                        className={queryStyles.button}
                        onClick={() => {
                          setTagSearchQuery(tagSearchDraft.trim());
                        }}
                      >
                        Search
                      </button>
                    </div>
                    <div className={queryStyles['starred-tag-filter-row']}>
                      <select
                        className={queryStyles['tag-filter-select']}
                        value={selectedTagFilter}
                        onChange={(event) => {
                          setSelectedTagFilter(event.currentTarget.value);
                        }}
                      >
                        <option value="all">All Saved</option>
                        {allTags.map((tag) => (
                          <option key={tag} value={tag}>{tag}</option>
                        ))}
                      </select>
                    </div>
                  </div>
                </div>

                <div className={queryStyles['results-shell']}>
                  <div className={queryStyles['results-tabs']}>
                    <div className={queryStyles['results-tabs-left']}>
                      <div className={queryStyles['starred-title']}>Saved Cards ({searchedSavedIds.length})</div>
                    </div>
                  </div>
                  <div className={queryStyles.results}>
                    {searchedSavedIds.map(renderStarredCard)}
                    {searchedSavedIds.length === 0 && (
                      <div className={queryStyles['end-of-results']}>
                        {tagSearchQuery
                          ? `No saved cards match "${tagSearchQuery}"${selectedTagFilter === 'all' ? '' : ` in tag "${selectedTagFilter}"`}`
                          : 'No saved cards for this tag'}
                      </div>
                    )}
                  </div>
                </div>

                <div className={`${queryStyles['card-panel']} ${queryStyles['starred-card-panel']}`}>
                  <div className={`${queryStyles['card-actions-row']} ${queryStyles['starred-actions-row']}`}>
                    <button
                      type="button"
                      className={`${queryStyles['toolbar-action']} ${queryStyles['starred-action-button']}`}
                      onClick={() => {
                        toggleSelectedCardStar();
                      }}
                      disabled={!selectedCard}
                    >
                      {isSelectedCardStarred ? 'Unstar' : 'Star'}
                    </button>
                    <button
                      type="button"
                      className={`${queryStyles['toolbar-action']} ${queryStyles['starred-action-button']}`}
                      onClick={openSelectedCardInSearch}
                      disabled={!selectedCard}
                    >
                      Open in Search
                    </button>
                    <button
                      type="button"
                      className={`${queryStyles['toolbar-action']} ${queryStyles['starred-action-button']}`}
                      onClick={() => setIsTagPickerOpen((open) => !open)}
                      disabled={!selectedCard}
                    >
                      Tag
                    </button>
                    {!!selectedPreference?.customTag && (
                      <div className={queryStyles['active-tag-badge']}>
                        Tag: {selectedPreference.customTag}
                      </div>
                    )}
                  </div>

                  {isTagPickerOpen && (
                    <div className={queryStyles['tag-picker']}>
                      <select
                        className={queryStyles['tag-picker-select']}
                        value={allTags.includes(tagDraft) ? tagDraft : ''}
                        onChange={(event) => {
                          setTagDraft(event.currentTarget.value);
                        }}
                      >
                        <option value="">Select existing tag</option>
                        {allTags.map((tag) => (
                          <option key={tag} value={tag}>{tag}</option>
                        ))}
                      </select>
                      <input
                        type="text"
                        className={queryStyles['tag-picker-input']}
                        value={tagDraft}
                        onChange={(event) => {
                          setTagDraft(event.currentTarget.value);
                        }}
                        placeholder="Create or edit custom tag"
                      />
                      <div className={queryStyles['tag-picker-actions']}>
                        <button
                          type="button"
                          className={queryStyles['toolbar-action']}
                          onClick={() => {
                            saveSelectedCardTag();
                          }}
                          disabled={!selectedCard}
                        >
                          Save Tag
                        </button>
                        <button
                          type="button"
                          className={queryStyles['toolbar-action']}
                          onClick={() => {
                            setTagDraft('');
                            saveSelectedCardTag('');
                          }}
                          disabled={!selectedCard}
                        >
                          Clear Tag
                        </button>
                      </div>
                    </div>
                  )}

                  <CardDetail
                    card={selectedCardData}
                    downloadUrls={downloadUrls}
                    onCardSave={(updatedCard) => {
                      setCards((previous) => ({ ...previous, [updatedCard.id]: updatedCard }));
                      updateCardPreferenceSnapshot(updatedCard);
                      refreshPreferences();
                    }}
                  />
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </>
  );
};

export default StarredPage;
