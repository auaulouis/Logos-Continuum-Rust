/* eslint-disable jsx-a11y/anchor-is-valid */
import {
  useState, useEffect, useCallback, useMemo, useRef,
} from 'react';
import Head from 'next/head';
import { RangeKeyDict } from 'react-date-range';
import { useRouter } from 'next/router';
import { format } from 'date-fns';
import Link from 'next/link';
import mixpanel from 'mixpanel-browser';
import { useSession } from 'next-auth/react';
import StyleSelect from '../components/StyleSelect';
import pageStyles from '../styles/index.module.scss';
import queryStyles from '../components/query/styles.module.scss';
import {
  InputBox, SearchResults, CardDetail,
} from '../components/query';
import type { CardDetailHandle } from '../components/query/CardDetail';
import * as apiService from '../services/api';
import { SearchResult, Card } from '../lib/types';
import {
  applySavedEdit,
} from '../lib/cardEdits';
import {
  CardPreference,
  getAllCardPreferences,
  getAllCustomTags,
  getCardPreference,
  setCardCustomTag,
  setCardStarred,
  subscribeToCardPreferences,
  updateCardPreferenceSnapshot,
} from '../lib/cardPreferences';
import {
  SideOption, sideOptions, divisionOptions, DivisionOption, yearOptions, YearOption, SchoolOption,
} from '../lib/constants';

type DebugLevel = 'info' | 'warn' | 'error';
type SearchTab = 'tag' | 'paragraph';
type DebugEntry = {
  id: number;
  at: number;
  level: DebugLevel;
  message: string;
};

const QueryPage = () => {
  type DebugPhase = 'closed' | 'open' | 'closing';
  const [query, setQuery] = useState(''); // current user input in the search box
  const [tabResults, setTabResults] = useState<Record<SearchTab, Array<SearchResult>>>({ tag: [], paragraph: [] });
  const [tabHasMoreResults, setTabHasMoreResults] = useState<Record<SearchTab, boolean>>({ tag: true, paragraph: true });
  const [tabCounts, setTabCounts] = useState<Record<SearchTab, number>>({ tag: -1, paragraph: -1 });
  const [tabCountsPartial, setTabCountsPartial] = useState<Record<SearchTab, boolean>>({ tag: false, paragraph: false });
  const [activeTab, setActiveTab] = useState<SearchTab>('tag');
  const [tabLoaded, setTabLoaded] = useState<Record<SearchTab, boolean>>({ tag: false, paragraph: false });
  const [cards, setCards] = useState<Record<string, any>>({}); // map of IDs to currently retrieved cards
  const [selectedCard, setSelectedCard] = useState('');
  const [loading, setLoading] = useState(false);
  const [searchDurationsMs, setSearchDurationsMs] = useState<Record<SearchTab, number>>({ tag: 0, paragraph: 0 });
  const [schools, setSchools] = useState<Array<SchoolOption>>([]); // list of schoools returned from the API
  const router = useRouter();
  const { query: routerQuery } = router;
  const {
    search: urlSearch,
    start_date,
    end_date,
    exclude_sides,
    exclude_division,
    exclude_years,
    exclude_schools,
    cite_match,
    use_personal,
    open_card,
    edit_card,
  } = routerQuery;
  const [downloadUrls, setDownloadUrls] = useState<Array<string>>([]);
  const [editRequest, setEditRequest] = useState(0);
  const [isCardEditing, setIsCardEditing] = useState(false);
  const [showCopiedToast, setShowCopiedToast] = useState(false);
  const [cardPreferences, setCardPreferences] = useState<Record<string, CardPreference>>({});
  const [isTagPickerOpen, setIsTagPickerOpen] = useState(false);
  const [customTagDraft, setCustomTagDraft] = useState('');
  const [debugPhase, setDebugPhase] = useState<DebugPhase>('closed');
  const [debugEntries, setDebugEntries] = useState<DebugEntry[]>([
    { id: 1, at: Date.now(), level: 'info', message: 'Query debug console initialized' },
  ]);
  const debugLogElement = useRef<HTMLDivElement | null>(null);
  const debugCloseTimer = useRef<number | null>(null);
  const searchDebounceTimer = useRef<number | null>(null);
  const activeSearchBatchRef = useRef(0);
  const searchControllersRef = useRef<Record<SearchTab, AbortController | null>>({ tag: null, paragraph: null });
  const deepLinkHandledRef = useRef('');
  const cardDetailRef = useRef<CardDetailHandle | null>(null);
  const selectedCardData = cards[selectedCard] as Card | undefined;
  const selectedCardPreference = selectedCard ? cardPreferences[selectedCard] : undefined;
  const isSelectedCardStarred = Boolean(selectedCardPreference?.starred);
  const allCustomTags = useMemo(() => getAllCustomTags(), [cardPreferences]);

  const isDebugOpen = debugPhase === 'open';
  const isDebugRendered = debugPhase !== 'closed';

  const closeDebugConsole = useCallback(() => {
    if (debugPhase === 'closing' || debugPhase === 'closed') {
      return;
    }
    setDebugPhase('closing');
    if (debugCloseTimer.current !== null) {
      window.clearTimeout(debugCloseTimer.current);
    }
    debugCloseTimer.current = window.setTimeout(() => {
      setDebugPhase('closed');
      debugCloseTimer.current = null;
    }, 220);
  }, [debugPhase]);

  const openDebugConsole = useCallback(() => {
    if (debugCloseTimer.current !== null) {
      window.clearTimeout(debugCloseTimer.current);
      debugCloseTimer.current = null;
    }
    setDebugPhase('open');
  }, []);

  const toggleDebugConsole = useCallback(() => {
    if (debugPhase === 'open') {
      closeDebugConsole();
    } else {
      openDebugConsole();
    }
  }, [debugPhase, closeDebugConsole, openDebugConsole]);

  const addDebugEntry = useCallback((level: DebugLevel, message: string) => {
    setDebugEntries((prev) => {
      const next: DebugEntry[] = [...prev, {
        id: Date.now() + Math.floor(Math.random() * 1000),
        at: Date.now(),
        level,
        message,
      }];
      return next.slice(-140);
    });
  }, []);

  const formattedDebugEntries = useMemo(() => debugEntries.map((entry) => {
    const timestamp = new Date(entry.at).toLocaleTimeString();
    return {
      ...entry,
      line: `[${timestamp}] ${entry.level.toUpperCase()} ${entry.message}`,
    };
  }), [debugEntries]);

  const onCopyDebugLogs = useCallback(async () => {
    const payload = formattedDebugEntries.map((entry) => entry.line).join('\n');
    if (!payload) {
      addDebugEntry('warn', 'Copy logs skipped: no logs to copy');
      return;
    }

    try {
      await navigator.clipboard.writeText(payload);
      addDebugEntry('info', `Copied ${formattedDebugEntries.length} log lines to clipboard`);
    } catch (error) {
      addDebugEntry('error', 'Failed to copy logs to clipboard');
    }
  }, [formattedDebugEntries, addDebugEntry]);

  const showCopiedMessage = useCallback(() => {
    setShowCopiedToast(true);
    addDebugEntry('info', 'Copied card content');
    window.setTimeout(() => {
      setShowCopiedToast(false);
    }, 1500);
  }, [addDebugEntry]);

  const onCopyCard = useCallback(async () => {
    if (!selectedCard) return;

    const copied = await cardDetailRef.current?.copyToClipboard();
    if (copied) {
      showCopiedMessage();
    } else {
      addDebugEntry('warn', 'Copy card failed');
    }
  }, [selectedCard, showCopiedMessage, addDebugEntry]);

  const refreshCardPreferences = useCallback(() => {
    setCardPreferences(getAllCardPreferences());
  }, []);

  // set the initial value of the filters based on the URL
  const urlSelectedSides = sideOptions.filter((side) => { return !exclude_sides?.includes(side.name); });
  const urlSelectedDivision = divisionOptions.filter((division) => { return !exclude_division?.includes(division.value); });
  const urlSelectedYears = yearOptions.filter((year) => { return !exclude_years?.includes(year.name); });
  const urlSelectedSchools = schools.filter((school) => { return !exclude_schools?.includes(school.name); });

  const [dateRange, setDateRange] = useState({
    startDate: new Date(),
    endDate: new Date(),
    key: 'selection',
  });

  const { data: session, status } = useSession();

  /**
   * Load the list of schools from the API on page load.
   */
  useEffect(() => {
    apiService.getSchools().then((schools) => {
      const { colleges } = schools;
      setSchools(colleges.map((college: string, i: number) => ({ name: college, id: i })));
    });
  }, []);

  useEffect(() => {
   // mixpanel.track('Page View', {
     // page: 'Home',
   // });
    addDebugEntry('info', 'Query page mounted');
    refreshCardPreferences();
  }, [addDebugEntry, refreshCardPreferences]);

  useEffect(() => {
    const unsubscribe = subscribeToCardPreferences(() => {
      refreshCardPreferences();
    });

    return unsubscribe;
  }, [refreshCardPreferences]);

  useEffect(() => {
    if (!selectedCard) {
      setIsTagPickerOpen(false);
      setCustomTagDraft('');
      return;
    }

    const currentPreference = getCardPreference(selectedCard);
    setCustomTagDraft(currentPreference?.customTag || '');
  }, [selectedCard, cardPreferences]);

  useEffect(() => {
    if (debugLogElement.current) {
      debugLogElement.current.scrollTop = debugLogElement.current.scrollHeight;
    }
  }, [formattedDebugEntries, isDebugOpen]);

  useEffect(() => () => {
    if (debugCloseTimer.current !== null) {
      window.clearTimeout(debugCloseTimer.current);
    }
    if (searchDebounceTimer.current !== null) {
      window.clearTimeout(searchDebounceTimer.current);
    }
    searchControllersRef.current.tag?.abort();
    searchControllersRef.current.paragraph?.abort();
  }, []);

  /**
    * Updates the specified fields or remove them from the URL.
    * Will trigger a new search if the query is different from the last query.
    */
  const updateUrl = (params: {[key: string]: string | undefined}, reset?: string[]) => {
    const query: Record<string, string> = {
      ...(params.search || urlSearch) && { search: params.search ? params.search : urlSearch as string },
      ...(params.start_date || start_date) && { start_date: params.start_date ? params.start_date : start_date as string },
      ...(params.end_date || end_date) && { end_date: params.end_date ? params.end_date : end_date as string },
      ...(params.exclude_sides || exclude_sides) && { exclude_sides: params.exclude_sides ? params.exclude_sides : exclude_sides as string },
      ...(params.exclude_division || exclude_division) && { exclude_division: params.exclude_division ? params.exclude_division : exclude_division as string },
      ...(params.exclude_years || exclude_years) && { exclude_years: params.exclude_years ? params.exclude_years : exclude_years as string },
      ...(params.exclude_schools || exclude_schools) && { exclude_schools: params.exclude_schools ? params.exclude_schools : exclude_schools as string },
      ...(params.cite_match || cite_match) && { cite_match: params.cite_match ? params.cite_match : cite_match as string },
      ...(params.use_personal || use_personal) && { use_personal: params.use_personal ? params.use_personal : use_personal as string },
    };
    for (const key of reset || []) {
      delete query[key];
    }
    router.push({
      pathname: '/query',
      query,
    });
    // mixpanel.track('Search', query);
  };

  /**
    * Updates the date range and triggers a new search.
    */
  const handleSelect = (ranges: RangeKeyDict) => {
    if (urlSearch) {
      if ((ranges.selection.endDate?.getTime() || 0) - (ranges.selection.startDate?.getTime() || 0) !== 0) {
        updateUrl({
          start_date: format((ranges.selection.startDate as Date), 'yyyy-MM-dd'),
          end_date: format((ranges.selection.endDate as Date), 'yyyy-MM-dd'),
        });
      } else {
        const start = ranges.selection.startDate || (start_date && start_date.length > 2 ? new Date(start_date as string) : new Date());
        const end = ranges.selection.endDate || (end_date && end_date.length > 2 ? new Date(end_date as string) : new Date());
        start.setUTCHours(12, 0, 0, 0);
        end.setUTCHours(12, 0, 0, 0);

        setDateRange((prev) => {
          return {
            ...prev,
            startDate: start,
            endDate: end,
          };
        });
      }
    }
  };

  const resetDate = () => {
    updateUrl({}, ['start_date', 'end_date']);
    setDateRange({
      startDate: new Date(),
      endDate: new Date(),
      key: 'selection',
    });
  };

  const resetSchools = () => {
    if (urlSelectedSchools.length !== schools.length) {
      updateUrl({}, ['exclude_schools']);
    } else {
      updateUrl({ exclude_schools: schools.map((school) => school.name).join(',') });
    }
  };

  const onSearch = async () => {
    const trimmedQuery = query.trim();
    const citeTokenMatch = trimmedQuery.match(/(?:^|\s)cite\s*:\s*(.+)$/i);
    const citeMatchValue = citeTokenMatch?.[1]?.trim() || '';
    const searchWithoutCite = citeTokenMatch
      ? trimmedQuery.slice(0, citeTokenMatch.index).trim()
      : trimmedQuery;

    if (!trimmedQuery) {
      updateUrl({}, ['search', 'cite_match']);
      return;
    }

    updateUrl({
      ...(searchWithoutCite && { search: encodeURI(searchWithoutCite) }),
      ...(citeMatchValue && { cite_match: encodeURI(citeMatchValue) }),
    }, [
      ...(!searchWithoutCite ? ['search'] : []),
      ...(!citeMatchValue ? ['cite_match'] : []),
    ]);
  };

  const cancelTabSearch = useCallback((tab: SearchTab) => {
    const controller = searchControllersRef.current[tab];
    if (controller) {
      controller.abort();
      searchControllersRef.current[tab] = null;
    }
  }, []);

  const isAbortError = (error: unknown): boolean => {
    if (typeof error !== 'object' || error === null) {
      return false;
    }

    const maybeError = error as { name?: string; code?: string };
    return maybeError.name === 'AbortError' || maybeError.code === 'ERR_CANCELED';
  };

  const searchRequest = async (
    tab: SearchTab,
    searchText = '',
    page = 0,
    options: { batchId?: number; cancelPrevious?: boolean } = {},
  ): Promise<boolean> => {
    const { batchId = activeSearchBatchRef.current, cancelPrevious = true } = options;
    const c = Math.max(0, page) * 30;
    const startedAt = performance.now();
    if (cancelPrevious) {
      cancelTabSearch(tab);
    }
    const controller = new AbortController();
    searchControllersRef.current[tab] = controller;
    addDebugEntry('info', `Search requested: "${searchText}" [${tab}] (cursor ${c})`);
    try {
      const response = await apiService.search(searchText, c, {
        match_mode: tab,
        ...(start_date) && { start_date: Math.floor(new Date(start_date as string).getTime() / 1000) },
        ...(end_date) && { end_date: Math.floor(new Date(end_date as string).getTime() / 1000) },
        ...(exclude_sides) && { exclude_sides },
        ...(exclude_division) && { exclude_division },
        ...(exclude_years) && { exclude_years },
        ...(exclude_schools) && { exclude_schools },
        ...(cite_match) && { cite_match },
        ...(use_personal) && { use_personal },
        ...!!(session && session.accessToken) && { access_token: session.accessToken },
      }, 30, { signal: controller.signal });

      if (controller.signal.aborted || batchId !== activeSearchBatchRef.current) {
        return false;
      }
      const {
        results: responseResults,
        cursor,
        totalCount,
        hasMore,
        countIsPartial,
      } = response;

      setTabResults((prev) => ({ ...prev, [tab]: responseResults }));
      setTabHasMoreResults((prev) => ({ ...prev, [tab]: Boolean(hasMore) }));
      setTabCounts((prev) => ({ ...prev, [tab]: Number.isFinite(totalCount) ? Number(totalCount) : responseResults.length }));
      setTabCountsPartial((prev) => ({ ...prev, [tab]: Boolean(countIsPartial) }));
      setSearchDurationsMs((prev) => ({ ...prev, [tab]: performance.now() - startedAt }));
      addDebugEntry('info', `Search response: ${responseResults.length} results for [${tab}] (next cursor ${cursor}, has_more=${Boolean(hasMore)}, partial_count=${Boolean(countIsPartial)})`);
      return true;
    } catch (error) {
      if (isAbortError(error)) {
        return false;
      }
      const message = error instanceof Error ? error.message : 'Search request failed';
      addDebugEntry('error', message);
      setTabHasMoreResults((prev) => ({ ...prev, [tab]: false }));
      return false;
    } finally {
      if (searchControllersRef.current[tab] === controller) {
        searchControllersRef.current[tab] = null;
      }
    }
  };

  const loadPage = async (tab: SearchTab, page: number): Promise<boolean> => {
    if ((urlSearch && urlSearch.length > 0) || cite_match) {
      return searchRequest(tab, decodeURI(urlSearch as string || ''), page, {
        batchId: activeSearchBatchRef.current,
      });
    }
    return false;
  };

  const handleTabChange = async (tab: SearchTab) => {
    setActiveTab(tab);
    // Lazy load the tab if not yet loaded
    if (!tabLoaded[tab] && (urlSearch || cite_match)) {
      setLoading(true);
      try {
        await searchRequest(tab, decodeURI(urlSearch as string || ''), 0, {
          batchId: activeSearchBatchRef.current,
          cancelPrevious: false,
        });
        setTabLoaded((prev) => ({ ...prev, [tab]: true }));
      } finally {
        setLoading(false);
      }
    }
  };

  // triggered for any changes in the URL
  useEffect(() => {
    // initiates a new search if the query exists
    if (status !== 'loading' && ((urlSearch && urlSearch.length > 0) || cite_match)) {
      const decodedQuery = decodeURI(urlSearch as string || '');
      const decodedCiteMatch = cite_match ? decodeURI(cite_match as string) : '';
      const batchId = activeSearchBatchRef.current + 1;
      activeSearchBatchRef.current = batchId;
      if (searchDebounceTimer.current !== null) {
        window.clearTimeout(searchDebounceTimer.current);
      }
      setQuery(`${decodedQuery}${decodedCiteMatch ? ` cite:${decodedCiteMatch}` : ''}`.trim());
      setTabResults({ tag: [], paragraph: [] });
      setTabHasMoreResults({ tag: true, paragraph: true });
      setTabCounts({ tag: -1, paragraph: -1 });
      setTabCountsPartial({ tag: false, paragraph: false });
      setSearchDurationsMs({ tag: 0, paragraph: 0 });
      setActiveTab('tag');
      setTabLoaded({ tag: false, paragraph: false });

      searchDebounceTimer.current = window.setTimeout(() => {
        (async () => {
          setLoading(true);
          try {
            // Only search the active tab initially - other tab loads lazily on switch
            await searchRequest('tag', decodedQuery, 0, { batchId });
            setTabLoaded({ tag: true, paragraph: false });
          } finally {
            if (batchId === activeSearchBatchRef.current) {
              setLoading(false);
            }
          }
        })();
      }, 140);
    } else if (status !== 'loading') {
      activeSearchBatchRef.current += 1;
      cancelTabSearch('tag');
      cancelTabSearch('paragraph');
      setLoading(false);
    }

    // update the date range based on changes to the URL
    if (start_date && end_date) {
      const start = new Date(start_date as string);
      const end = new Date(end_date as string);
      start.setUTCHours(12, 0, 0, 0);
      end.setUTCHours(12, 0, 0, 0);

      setDateRange((prev) => {
        return {
          ...prev,
          startDate: start,
          endDate: end,
        };
      });
    }
  }, [routerQuery, status, cancelTabSearch]);

  const getCard = async (id: string): Promise<Card | undefined> => {
    if (cards[id]) {
      return cards[id] as Card;
    }

    try {
      const card = await apiService.getCard(id);
      const hydratedCard = applySavedEdit(card);
      setCards((c) => { return { ...c, [id]: hydratedCard }; });
      updateCardPreferenceSnapshot(hydratedCard);
      addDebugEntry('info', `Loaded card: ${id}`);
      return hydratedCard;
    } catch (error) {
      const message = error instanceof Error ? error.message : `Failed to load card: ${id}`;
      addDebugEntry('error', message);
      return undefined;
    }
  };

  useEffect(() => {
    if (selectedCard) {
      getCard(selectedCard);
    }
  }, [selectedCard]);

  useEffect(() => {
    if (!selectedCardData) {
      setDownloadUrls([]);
      return;
    }

    const selectedUrls = selectedCardData.download_url;
    if (selectedUrls) {
      setDownloadUrls(Array.isArray(selectedUrls) ? selectedUrls : [selectedUrls]);
    } else {
      setDownloadUrls([]);
    }
  }, [selectedCard, selectedCardData]);

  useEffect(() => {
    if (!router.isReady) {
      return;
    }

    const cardId = typeof open_card === 'string' ? open_card.trim() : '';
    if (!cardId) {
      return;
    }

    const shouldEdit = edit_card === 'true';
    const deepLinkKey = `${cardId}|${shouldEdit}`;

    if (deepLinkHandledRef.current === deepLinkKey) {
      return;
    }

    deepLinkHandledRef.current = deepLinkKey;
    setSelectedCard(cardId);

    if (shouldEdit) {
      setEditRequest((count) => count + 1);
    }
  }, [router.isReady, open_card, edit_card]);

  const onSideSelect = (sides: SideOption[]) => {
    if (sides.length === 1) {
      updateUrl({ exclude_sides: sideOptions.filter((opt) => !sides.find((side) => side.value === opt.value)).map((opt) => opt.name).join('') });
    } else if (sides.length === 2) {
      updateUrl({}, ['exclude_sides']);
    }
  };

  const onDivisionSelect = (divisions: DivisionOption[]) => {
    if (divisions.length < divisionOptions.length) {
      updateUrl({ exclude_division: divisionOptions.filter((opt) => !divisions.find((div) => div.value === opt.value)).map((opt) => opt.value).join(',') });
    } else {
      updateUrl({}, ['exclude_division']);
    }
  };

  const onYearSelect = (years: YearOption[]) => {
    if (years.length < yearOptions.length) {
      updateUrl({ exclude_years: yearOptions.filter((opt) => !years.find((div) => div.name === opt.name)).map((opt) => opt.name).join(',') });
    } else {
      updateUrl({}, ['exclude_years']);
    }
  };

  const onSchoolSelect = (s: SchoolOption[]) => {
    if (s.length < schools.length) {
      updateUrl({ exclude_schools: schools.filter((opt) => !s.find((school) => school.name === opt.name)).map((opt) => opt.name).join(',') });
    } else {
      updateUrl({}, ['exclude_schools']);
    }
  };

  const togglePersonal = () => {
    if (use_personal === 'true') {
      updateUrl({ }, ['use_personal']);
    } else {
      updateUrl({ use_personal: 'true' });
    }
  };

  const toggleSelectedCardStar = async () => {
    if (!selectedCard) return;

    const card = selectedCardData || await getCard(selectedCard);
    if (!card) return;

    const nextStarred = !isSelectedCardStarred;
    setCardStarred(card, nextStarred);
    addDebugEntry('info', `${nextStarred ? 'Starred' : 'Unstarred'} card: ${card.id}`);
    refreshCardPreferences();
  };

  const saveSelectedCardTag = async (nextTagValue?: string) => {
    if (!selectedCard) return;

    const card = selectedCardData || await getCard(selectedCard);
    if (!card) return;

    const normalizedTag = (nextTagValue ?? customTagDraft).trim();
    setCardCustomTag(card, normalizedTag);
    refreshCardPreferences();
    setIsTagPickerOpen(false);
    addDebugEntry('info', `${normalizedTag ? `Saved custom tag "${normalizedTag}"` : 'Cleared custom tag'} for card: ${card.id}`);
  };

  return (
    <>
      <Head>
        <title>Logos Continuum: A Debate Search Engine</title>
        <meta name="description" content="Search debate cards with Logos Continuum" />
        <link rel="icon" href="/favicon.ico" />
      </Head>
      <div className={pageStyles.container}>
        <div className={pageStyles['corner-controls']}>
          <button
            type="button"
            className={pageStyles['bug-report-button']}
            aria-label="Toggle debug console"
            onClick={toggleDebugConsole}
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              viewBox="0 -960 960 960"
              aria-hidden="true"
              className={pageStyles['bug-report-icon']}
            >
              <path d="M480-200q66 0 113-47t47-113v-160q0-66-47-113t-113-47q-66 0-113 47t-47 113v160q0 66 47 113t113 47Zm-80-120h160v-80H400v80Zm0-160h160v-80H400v80Zm80 40Zm0 320q-65 0-120.5-32T272-240H160v-80h84q-3-20-3.5-40t-.5-40h-80v-80h80q0-20 .5-40t3.5-40h-84v-80h112q14-23 31.5-43t40.5-35l-64-66 56-56 86 86q28-9 57-9t57 9l88-86 56 56-66 66q23 15 41.5 34.5T688-640h112v80h-84q3 20 3.5 40t.5 40h80v80h-80q0 20-.5 40t-3.5 40h84v80H688q-32 56-87.5 88T480-120Z" />
            </svg>
          </button>
        </div>
        {isDebugRendered && (
          <div
            className={`${pageStyles['debug-console']} ${debugPhase === 'closing' ? pageStyles['debug-console-closing'] : ''}`}
            role="dialog"
            aria-label="Debug console"
          >
            <div className={pageStyles['debug-console-header']}>
              <span>logs@logos-continuum:~$</span>
              <div className={pageStyles['debug-console-actions']}>
                <button
                  type="button"
                  className={pageStyles['debug-console-btn']}
                  onClick={onCopyDebugLogs}
                >
                  copy logs
                </button>
                <button
                  type="button"
                  className={pageStyles['debug-console-btn']}
                  onClick={() => setDebugEntries([])}
                >
                  clear
                </button>
                <button
                  type="button"
                  className={pageStyles['debug-console-btn']}
                  onClick={closeDebugConsole}
                >
                  close
                </button>
              </div>
            </div>
            <div ref={debugLogElement} className={pageStyles['debug-console-body']}>
              {formattedDebugEntries.length === 0 && (
                <div className={pageStyles['debug-line-muted']}>[empty] no events yet</div>
              )}
              {formattedDebugEntries.map((entry) => (
                <div key={entry.id} className={`${pageStyles['debug-line']} ${pageStyles[`debug-line-${entry.level}`]}`}>
                  {entry.line}
                </div>
              ))}
            </div>
          </div>
        )}
        <div className={pageStyles.foreground}>
          <div className="query-shell">
            <div className="logo query-logo">
              <Link href="/" passHref><a><h1 className={pageStyles.logo}>Logos Continuum</h1></a></Link>
              <div className={queryStyles['top-controls']}>
                {!isCardEditing && (
                  <button
                    type="button"
                    className={queryStyles['toolbar-action']}
                    onClick={() => setEditRequest((n) => n + 1)}
                    disabled={!selectedCard}
                  >
                    <img
                      src="/edit_24dp_E3E3E3_FILL0_wght400_GRAD0_opsz24.png"
                      alt="Edit card"
                      className={queryStyles['icon-image']}
                    />
                    Edit
                  </button>
                )}
                {!isCardEditing && (
                  <>
                    <button
                      type="button"
                      className={queryStyles['toolbar-action']}
                      onClick={() => {
                        toggleSelectedCardStar();
                      }}
                      disabled={!selectedCard}
                    >
                      {isSelectedCardStarred ? '★ Starred' : '☆ Star'}
                    </button>
                    <button
                      type="button"
                      className={queryStyles['toolbar-action']}
                      onClick={() => setIsTagPickerOpen((open) => !open)}
                      disabled={!selectedCard}
                    >
                      Tag
                    </button>
                    <button
                      type="button"
                      className={queryStyles['toolbar-action']}
                      onClick={() => {
                        onCopyCard();
                      }}
                      disabled={!selectedCard}
                    >
                      <img
                        src="/copy_all_24dp_E3E3E3_FILL0_wght400_GRAD0_opsz24.png"
                        alt="Copy card"
                        className={queryStyles['icon-image']}
                      />
                      Copy
                    </button>
                    <StyleSelect />
                  </>
                )}
              </div>
            </div>

            <div className="query-page">
              <div className="page-row">
                <InputBox
                  value={query}
                  onChange={setQuery}
                  onSearch={onSearch}
                  loading={loading}
                />
              </div>

              <div className="page-row">
                <SearchResults
                  tabResults={tabResults}
                  tabCounts={tabCounts}
                  tabCountsPartial={tabCountsPartial}
                  searchDurationsMs={searchDurationsMs}
                  query={query}
                  setSelected={setSelectedCard}
                  cards={cards}
                  getCard={getCard}
                  loadPage={loadPage}
                  setDownloadUrls={setDownloadUrls}
                  tabHasMoreResults={tabHasMoreResults}
                  cardPreferences={cardPreferences}
                  activeTab={activeTab}
                  onTabChange={handleTabChange}
                  loading={loading}
                />
                <div className={queryStyles['card-panel']}>
                  {!isCardEditing && isTagPickerOpen && (
                    <div className={queryStyles['tag-picker']}>
                      <select
                        className={queryStyles['tag-picker-select']}
                        value={allCustomTags.includes(customTagDraft) ? customTagDraft : ''}
                        onChange={(event) => {
                          setCustomTagDraft(event.currentTarget.value);
                        }}
                      >
                        <option value="">Select existing tag</option>
                        {allCustomTags.map((tagName) => (
                          <option key={tagName} value={tagName}>{tagName}</option>
                        ))}
                      </select>
                      <input
                        type="text"
                        className={queryStyles['tag-picker-input']}
                        value={customTagDraft}
                        onChange={(event) => {
                          setCustomTagDraft(event.currentTarget.value);
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
                            setCustomTagDraft('');
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
                    ref={cardDetailRef}
                    card={cards[selectedCard]}
                    downloadUrls={downloadUrls}
                    externalEditRequest={editRequest}
                    onEditModeChange={setIsCardEditing}
                    editorRightActions={isCardEditing ? (
                      <>
                        <button
                          type="button"
                          className={queryStyles['toolbar-action']}
                          onClick={() => {
                            onCopyCard();
                          }}
                          disabled={!selectedCard}
                        >
                          <img
                            src="/copy_all_24dp_E3E3E3_FILL0_wght400_GRAD0_opsz24.png"
                            alt="Copy card"
                            className={queryStyles['icon-image']}
                          />
                          Copy
                        </button>
                        <StyleSelect />
                      </>
                    ) : undefined}
                    onCardSave={(updatedCard) => {
                      setCards((prev) => ({ ...prev, [updatedCard.id]: updatedCard }));
                      updateCardPreferenceSnapshot(updatedCard);
                      refreshCardPreferences();
                    }}
                  />
                </div>
              </div>
            </div>
          </div>
        </div>
        {showCopiedToast && <div className={queryStyles['copy-toast']}>Copied</div>}
      </div>
    </>
  );
};

export default QueryPage;
