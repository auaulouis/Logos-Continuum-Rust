import Head from 'next/head';
import Link from 'next/link';
import { useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import * as apiService from '../services/api';
import type { ParsedDocument, ParserSettings } from '../services/api';
import { AppContext } from '../lib/appContext';
import {
  getAllSavedCardEdits,
  getSavedCardEditsCount,
  saveCardEdit,
} from '../lib/cardEdits';
import {
  exportSavedEditsToDocx,
  resolveSourceDocumentLabelsFromCard,
} from '../lib/cardDocxExport';
import packageJson from '../package.json';
import styles from '../styles/settings.module.scss';
import indexStyles from '../styles/index.module.scss';

type MessageLevel = 'info' | 'error';
type DebugLevel = 'info' | 'warn' | 'error';
type DebugEntry = {
  id: number;
  at: number;
  level: DebugLevel;
  message: string;
};

const DEFAULT_PARSER_SETTINGS: ParserSettings = {
  use_parallel_processing: true,
  parser_card_workers: 1,
  local_parser_file_workers: 4,
  flush_enabled: true,
  flush_every_docs: 250,
};

const APP_VERSION = packageJson.version;

const SettingsPage = () => {
  type DebugPhase = 'closed' | 'open' | 'closing';
  const { theme, toggleTheme } = useContext(AppContext);
  const [message, setMessage] = useState('');
  const [messageLevel, setMessageLevel] = useState<MessageLevel>('info');

  const [clearIndexSelected, setClearIndexSelected] = useState(true);
  const [clearFilesSelected, setClearFilesSelected] = useState(false);
  const [isClearingIndex, setIsClearingIndex] = useState(false);

  const [parserSettings, setParserSettings] = useState<ParserSettings>(DEFAULT_PARSER_SETTINGS);
  const [parserSettingsError, setParserSettingsError] = useState('');
  const [isSavingParserSettings, setIsSavingParserSettings] = useState(false);
  const [savedEditsCount, setSavedEditsCount] = useState(0);
  const [isExportingSavedEdits, setIsExportingSavedEdits] = useState(false);

  const [documents, setDocuments] = useState<ParsedDocument[]>([]);
  const [documentsError, setDocumentsError] = useState('');
  const [documentsSearch, setDocumentsSearch] = useState('');
  const [showHiddenDocuments, setShowHiddenDocuments] = useState(true);
  const [isDocumentsBoxOpen, setIsDocumentsBoxOpen] = useState(false);
  const [isSelectMode, setIsSelectMode] = useState(false);
  const [selectedDocuments, setSelectedDocuments] = useState<string[]>([]);
  const [isDocumentsLoading, setIsDocumentsLoading] = useState(false);
  const [deleteInProgressKey, setDeleteInProgressKey] = useState<string | null>(null);
  const [isManualOpen, setIsManualOpen] = useState(false);
  const [debugPhase, setDebugPhase] = useState<DebugPhase>('closed');
  const [debugEntries, setDebugEntries] = useState<DebugEntry[]>([
    { id: 1, at: Date.now(), level: 'info', message: 'Settings debug console initialized' },
  ]);
  const debugLogElement = useRef<HTMLDivElement | null>(null);
  const debugCloseTimer = useRef<number | null>(null);

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
    } catch {
      addDebugEntry('error', 'Failed to copy logs to clipboard');
    }
  }, [formattedDebugEntries, addDebugEntry]);

  const updateMessage = (text: string, level: MessageLevel = 'info') => {
    setMessage(text);
    setMessageLevel(level);
  };

  const loadParserSettings = useCallback(async () => {
    setParserSettingsError('');
    try {
      const response = await apiService.getParserSettings();
      setParserSettings(response.settings);
      addDebugEntry('info', 'Parser settings loaded');
    } catch {
      setParserSettingsError('Failed to load parser settings.');
      addDebugEntry('error', 'Failed to load parser settings');
    }
  }, [addDebugEntry]);

  const loadParsedDocuments = useCallback(async () => {
    setIsDocumentsLoading(true);
    setDocumentsError('');
    try {
      const response = await apiService.getParsedDocuments();
      const docs = Array.isArray(response.documents) ? response.documents : [];
      docs.sort((a, b) => a.filename.localeCompare(b.filename));
      setDocuments(docs);
      addDebugEntry('info', `Loaded ${docs.length} parsed document(s)`);
    } catch {
      setDocumentsError('Failed to load parsed documents.');
      addDebugEntry('error', 'Failed to load parsed documents');
    } finally {
      setIsDocumentsLoading(false);
    }
  }, [addDebugEntry]);

  const refreshSavedEditsCount = useCallback(() => {
    setSavedEditsCount(getSavedCardEditsCount());
  }, []);

  const onExportSavedEdits = useCallback(async () => {
    const savedEdits = getAllSavedCardEdits();
    if (!savedEdits.length) {
      updateMessage('No saved edits available to export.', 'error');
      addDebugEntry('warn', 'Export skipped: no saved card edits');
      refreshSavedEditsCount();
      return;
    }

    setIsExportingSavedEdits(true);
    try {
      const hydratedEdits = await Promise.all(savedEdits.map(async (entry) => {
        if (entry.edit.sourceDocuments && entry.edit.sourceDocuments.length > 0) {
          return entry;
        }

        try {
          const remoteCard = await apiService.getCard(entry.cardId);
          const resolvedSources = resolveSourceDocumentLabelsFromCard({
            sourceUrls: remoteCard?.download_url || remoteCard?.s3_url,
            filename: remoteCard?.filename,
          });

          if (resolvedSources.length === 0) {
            return entry;
          }

          const nextEdit = {
            ...entry.edit,
            sourceDocuments: resolvedSources,
            cardIdentifier: entry.edit.cardIdentifier || remoteCard?.card_identifier,
          };

          saveCardEdit(entry.cardId, nextEdit);
          return {
            ...entry,
            edit: nextEdit,
          };
        } catch {
          return entry;
        }
      }));

      await exportSavedEditsToDocx(hydratedEdits);
      updateMessage(`Exported ${hydratedEdits.length} saved edits to .docx.`);
      addDebugEntry('info', `Exported ${hydratedEdits.length} saved card edits to DOCX`);
    } catch {
      updateMessage('Failed to export saved edits.', 'error');
      addDebugEntry('error', 'Failed to export saved card edits');
    } finally {
      refreshSavedEditsCount();
      setIsExportingSavedEdits(false);
    }
  }, [addDebugEntry, refreshSavedEditsCount]);

  useEffect(() => {
    loadParserSettings();
    loadParsedDocuments();
    refreshSavedEditsCount();
  }, [loadParserSettings, loadParsedDocuments, refreshSavedEditsCount]);

  useEffect(() => {
    if (debugLogElement.current) {
      debugLogElement.current.scrollTop = debugLogElement.current.scrollHeight;
    }
  }, [formattedDebugEntries, isDebugOpen]);

  useEffect(() => () => {
    if (debugCloseTimer.current !== null) {
      window.clearTimeout(debugCloseTimer.current);
    }
  }, []);

  const onSaveParserSettings = async () => {
    setIsSavingParserSettings(true);
    setParserSettingsError('');
    try {
      const payload: ParserSettings = {
        use_parallel_processing: !!parserSettings.use_parallel_processing,
        parser_card_workers: Math.max(1, Number(parserSettings.parser_card_workers) || 1),
        local_parser_file_workers: Math.max(1, Number(parserSettings.local_parser_file_workers) || 1),
        flush_enabled: !!parserSettings.flush_enabled,
        flush_every_docs: Math.max(1, Number(parserSettings.flush_every_docs) || 1),
      };
      const response = await apiService.updateParserSettings(payload);
      setParserSettings(response.settings);
      updateMessage('Parser settings saved.');
      addDebugEntry('info', 'Parser settings saved');
    } catch {
      setParserSettingsError('Failed to save parser settings.');
      updateMessage('Failed to save parser settings.', 'error');
      addDebugEntry('error', 'Failed to save parser settings');
    } finally {
      setIsSavingParserSettings(false);
    }
  };

  const onConfirmClearIndex = async () => {
    if (!clearIndexSelected && !clearFilesSelected) {
      return;
    }

    setIsClearingIndex(true);
    try {
      const actionNotes: string[] = [];
      let deletedFiles = 0;
      let failedFileDeletes = 0;

      if (clearIndexSelected) {
        await apiService.clearIndex();
        actionNotes.push('index cleared');
      }

      if (clearFilesSelected) {
        const response = await apiService.getParsedDocuments();
        const docs = Array.isArray(response.documents) ? response.documents : [];
        for (const document of docs) {
          if (!document.in_folder) continue;
          try {
            await apiService.deleteParsedDocument(document.filename, 'folder');
            deletedFiles += 1;
          } catch {
            failedFileDeletes += 1;
          }
        }
        actionNotes.push(`deleted ${deletedFiles} .docx file(s)`);
      }

      updateMessage(`${actionNotes.join(' + ')}${failedFileDeletes > 0 ? ` (${failedFileDeletes} file deletions failed)` : ''}`);
      addDebugEntry('info', `Clear action complete: ${actionNotes.join(' + ') || 'no-op'}`);
      await loadParsedDocuments();
    } catch {
      updateMessage('Failed to run clear action.', 'error');
      addDebugEntry('error', 'Failed to run clear action');
    } finally {
      setIsClearingIndex(false);
    }
  };

  const filteredDocuments = useMemo(() => {
    const queryText = documentsSearch.trim().toLowerCase();
    return documents.filter((document) => {
      if (!showHiddenDocuments && !document.in_index) {
        return false;
      }
      if (!queryText) {
        return true;
      }
      return document.filename.toLowerCase().includes(queryText);
    });
  }, [documents, documentsSearch, showHiddenDocuments]);

  const onDeleteDocument = async (document: ParsedDocument, target: 'index' | 'folder') => {
    const actionKey = `${document.filename}:${target}`;
    setDeleteInProgressKey(actionKey);
    try {
      await apiService.deleteParsedDocument(document.filename, target);
      updateMessage(
        target === 'index'
          ? `Removed ${document.filename} from parsed index.`
          : `Removed ${document.filename} from uploaded docs folder.`,
      );
      addDebugEntry('info', `Deleted ${document.filename} from ${target}`);
      await loadParsedDocuments();
    } catch {
      setDocumentsError('Failed to delete document for selected target.');
      updateMessage('Delete failed for selected document.', 'error');
      addDebugEntry('error', `Delete failed for ${document.filename} (${target})`);
    } finally {
      setDeleteInProgressKey(null);
    }
  };

  const toggleSelectedDocument = (filename: string) => {
    setSelectedDocuments((prev) => {
      if (prev.includes(filename)) {
        return prev.filter((item) => item !== filename);
      }
      return [...prev, filename];
    });
  };

  const onToggleSelectMode = () => {
    setIsSelectMode((prev) => !prev);
    setSelectedDocuments([]);
  };

  const onSelectAllVisible = () => {
    const visibleNames = filteredDocuments.map((document) => document.filename);
    setSelectedDocuments((prev) => {
      const areAllSelected = visibleNames.length > 0 && visibleNames.every((name) => prev.includes(name));
      if (areAllSelected) {
        return prev.filter((name) => !visibleNames.includes(name));
      }
      const merged = new Set([...prev, ...visibleNames]);
      return Array.from(merged);
    });
  };

  const onDeleteSelectedDocuments = async () => {
    if (selectedDocuments.length === 0) {
      return;
    }

    const selectedSet = new Set(selectedDocuments);
    const targets = documents.filter((document) => selectedSet.has(document.filename));
    setDeleteInProgressKey('bulk');
    setDocumentsError('');

    let updated = 0;
    for (const document of targets) {
      try {
        if (document.in_index) {
          await apiService.deleteParsedDocument(document.filename, 'index');
        }
        if (document.in_folder) {
          await apiService.deleteParsedDocument(document.filename, 'folder');
        }
        updated += 1;
      } catch {
        setDocumentsError(`Some selected documents could not be deleted (stopped at ${document.filename}).`);
        break;
      }
    }

    if (updated > 0) {
      updateMessage(`Deleted ${updated} selected document(s) from index/folder where available.`);
    }

    await loadParsedDocuments();
    setSelectedDocuments([]);
    setDeleteInProgressKey(null);
  };

  const openDocumentsBox = async () => {
    setIsDocumentsBoxOpen(true);
    await loadParsedDocuments();
  };

  const closeDocumentsBox = () => {
    if (deleteInProgressKey) {
      return;
    }
    setIsDocumentsBoxOpen(false);
    setDocumentsError('');
    setDocumentsSearch('');
    setShowHiddenDocuments(true);
    setIsSelectMode(false);
    setSelectedDocuments([]);
  };

  const closeManual = () => {
    setIsManualOpen(false);
  };

  return (
    <>
      <Head>
        <title>Settings | Logos Continuum</title>
        <meta name="description" content="Parser and document management settings" />
      </Head>
      <div className={styles.page}>
        <div className={styles.headerRow}>
          <Link href="/" passHref>
            <a className={styles.logoLink}>LOGOS CONTINUUM</a>
          </Link>
          <h1 className={styles.title}>Settings</h1>
        </div>

        {message && (
          <p className={messageLevel === 'error' ? styles.errorMessage : styles.infoMessage}>{message}</p>
        )}

        <div className={styles.cardsGrid}>
          <div className={styles.column}>
            <section className={styles.card}>
              <h2 className={styles.sectionTitle}>Appearance</h2>
              <p className={styles.meta}>Current theme: {theme === 'dark' ? 'Dark' : 'Light'}</p>
              <div className={styles.actions}>
                <button type="button" className={styles.secondaryBtn} onClick={toggleTheme}>
                  Switch to {theme === 'dark' ? 'Light' : 'Dark'} Mode
                </button>
              </div>
            </section>

            <section className={styles.card}>
              <h2 className={styles.sectionTitle}>Clear Parsed Cards</h2>
              <p className={styles.meta}>Choose what to clear.</p>

            <label className={styles.row}>
              <span>Clear parsed cards from index</span>
              <input
                type="checkbox"
                checked={clearIndexSelected}
                onChange={(event) => setClearIndexSelected(event.target.checked)}
                disabled={isClearingIndex}
              />
            </label>

            <label className={styles.row}>
              <span>Delete uploaded .docx files</span>
              <input
                type="checkbox"
                checked={clearFilesSelected}
                onChange={(event) => setClearFilesSelected(event.target.checked)}
                disabled={isClearingIndex}
              />
            </label>

              <div className={styles.actions}>
                <button
                  type="button"
                  className={styles.dangerBtn}
                  onClick={onConfirmClearIndex}
                  disabled={isClearingIndex || (!clearIndexSelected && !clearFilesSelected)}
                >
                  {isClearingIndex ? 'Clearing…' : 'Run Clear Action'}
                </button>
              </div>
            </section>
          </div>

          <div className={styles.column}>
            <section className={`${styles.card} ${styles.parserCard}`}>
              <h2 className={styles.sectionTitle}>Parser Settings</h2>
              {parserSettingsError && <p className={styles.errorMessage}>{parserSettingsError}</p>}

          <label className={styles.row}>
            <span>Use parallel processing</span>
            <input
              type="checkbox"
              checked={parserSettings.use_parallel_processing}
              onChange={(event) => setParserSettings((prev) => ({ ...prev, use_parallel_processing: event.target.checked }))}
              disabled={isSavingParserSettings}
            />
          </label>

          <label className={styles.row}>
            <span>Card workers (cores)</span>
            <input
              type="number"
              min={1}
              className={styles.numberInput}
              value={parserSettings.parser_card_workers}
              onChange={(event) => setParserSettings((prev) => ({ ...prev, parser_card_workers: Number(event.target.value) || 1 }))}
              disabled={isSavingParserSettings}
            />
          </label>

          <label className={styles.row}>
            <span>File workers (cores)</span>
            <input
              type="number"
              min={1}
              className={styles.numberInput}
              value={parserSettings.local_parser_file_workers}
              onChange={(event) => setParserSettings((prev) => ({ ...prev, local_parser_file_workers: Number(event.target.value) || 1 }))}
              disabled={isSavingParserSettings}
            />
          </label>

          <label className={styles.row}>
            <span>Enable periodic flush</span>
            <input
              type="checkbox"
              checked={parserSettings.flush_enabled}
              onChange={(event) => setParserSettings((prev) => ({ ...prev, flush_enabled: event.target.checked }))}
              disabled={isSavingParserSettings}
            />
          </label>

          <label className={styles.row}>
            <span>Flush every N documents</span>
            <input
              type="number"
              min={1}
              className={styles.numberInput}
              value={parserSettings.flush_every_docs}
              onChange={(event) => setParserSettings((prev) => ({ ...prev, flush_every_docs: Number(event.target.value) || 1 }))}
              disabled={isSavingParserSettings || !parserSettings.flush_enabled}
            />
          </label>

            <div className={styles.actions}>
              <button type="button" className={styles.primaryBtn} onClick={onSaveParserSettings} disabled={isSavingParserSettings}>
                {isSavingParserSettings ? 'Saving…' : 'Save Settings'}
              </button>
            </div>
            </section>

            <section className={styles.card}>
              <h2 className={styles.sectionTitle}>Export Saved Edits</h2>
              <p className={styles.meta}>Export locally saved card edits into a single .docx file.</p>
              <div className={styles.actions}>
                <button
                  type="button"
                  className={styles.primaryBtn}
                  onClick={onExportSavedEdits}
                  disabled={savedEditsCount === 0 || isExportingSavedEdits}
                >
                  {isExportingSavedEdits ? 'Exporting…' : `Export Saved Edits (${savedEditsCount})`}
                </button>
              </div>
            </section>
          </div>

          <div className={`${styles.column} ${styles.utilityGrid}`}>
            <section className={`${styles.card} ${styles.manageCard}`}>
              <h2 className={styles.sectionTitle}>Manage Documents</h2>
              <p className={styles.meta}>Open the document manager popup to search, select, and delete documents.</p>
              <div className={styles.actions}>
                <button type="button" className={styles.primaryBtn} onClick={openDocumentsBox}>
                  Open Manage Documents ({documents.length})
                </button>
              </div>
            </section>

            <section className={`${styles.card} ${styles.manualCard}`}>
              <h2 className={styles.sectionTitle}>Manual</h2>
              <p className={styles.meta}>Open the complete app guide for parsing, search, editing, export, and settings workflows.</p>
              <div className={styles.actions}>
                <button type="button" className={styles.secondaryBtn} onClick={() => setIsManualOpen(true)}>
                  Open Manual
                </button>
              </div>
            </section>

            <section className={`${styles.card} ${styles.aboutCard}`}>
              <h2 className={styles.sectionTitle}>About</h2>
              <p className={styles.meta}><strong>Version:</strong> {APP_VERSION}</p>
              <p className={styles.meta}><strong>Developer:</strong> auaulouis</p>
              <p className={styles.meta}>
                <strong>Open Source:</strong> MIT License. You are permitted to use, copy, modify, merge, publish,
                distribute, sublicense, and/or sell copies of the software, subject to the MIT license terms.
              </p>
            </section>

            <section className={`${styles.card} ${styles.debugCard}`}>
              <h2 className={styles.sectionTitle}>Debug Console</h2>
              <p className={styles.meta}>Open runtime logs directly from Settings.</p>
              <div className={styles.actions}>
                <button type="button" className={styles.secondaryBtn} onClick={toggleDebugConsole}>
                  {isDebugOpen ? 'Close Debug Console' : 'Open Debug Console'}
                </button>
              </div>
            </section>
          </div>
        </div>

        {isDebugRendered && (
          <div
            className={`${indexStyles['debug-console']} ${debugPhase === 'closing' ? indexStyles['debug-console-closing'] : ''}`}
            role="dialog"
            aria-label="Debug console"
          >
            <div className={indexStyles['debug-console-header']}>
              <span>logs@logos-continuum:~$</span>
              <div className={indexStyles['debug-console-actions']}>
                <button
                  type="button"
                  className={indexStyles['debug-console-btn']}
                  onClick={onCopyDebugLogs}
                >
                  copy logs
                </button>
                <button
                  type="button"
                  className={indexStyles['debug-console-btn']}
                  onClick={() => setDebugEntries([])}
                >
                  clear
                </button>
                <button
                  type="button"
                  className={indexStyles['debug-console-btn']}
                  onClick={closeDebugConsole}
                >
                  close
                </button>
              </div>
            </div>
            <div ref={debugLogElement} className={indexStyles['debug-console-body']}>
              {formattedDebugEntries.length === 0 && (
                <div className={indexStyles['debug-line-muted']}>[empty] no events yet</div>
              )}
              {formattedDebugEntries.map((entry) => (
                <div key={entry.id} className={`${indexStyles['debug-line']} ${indexStyles[`debug-line-${entry.level}`]}`}>
                  {entry.line}
                </div>
              ))}
            </div>
          </div>
        )}

        {isManualOpen && (
          <div className={styles.manualOverlay} role="presentation" onClick={closeManual}>
            <div
              className={styles.manualDialog}
              role="dialog"
              aria-modal="true"
              aria-label="App manual"
              onClick={(event) => event.stopPropagation()}
            >
              <div className={styles.manualHeader}>
                <h3 className={styles.manualTitle}>Logos Continuum Manual</h3>
                <button type="button" className={styles.secondaryBtn} onClick={closeManual}>Close</button>
              </div>

              <div className={styles.manualBody}>
                <section className={styles.manualSection}>
                  <h4>1) What Logos Continuum does</h4>
                  <p>
                    Logos Continuum is a local debate-card workflow app for parsing, searching, reviewing, editing,
                    and exporting evidence cards. The normal flow is: upload .docx files on Home, search on Query,
                    edit selected cards, then export saved edits from Settings to a .docx.
                  </p>
                </section>

                <section className={styles.manualSection}>
                  <h4>2) Home page: search + parse uploads</h4>
                  <ul>
                    <li><strong>Search from Home:</strong> type a query and press Enter or click <em>Submit</em> to open Query with that search.</li>
                    <li><strong>Open Settings:</strong> use the <em>Settings</em> button in the top-right corner.</li>
                    <li><strong>Debug console:</strong> click the bug icon to open runtime logs; you can <em>copy logs</em>, <em>clear</em>, and <em>close</em>.</li>
                    <li><strong>Upload parsing:</strong> drag/drop .docx files or click the drop zone to choose files.</li>
                    <li><strong>Multi-file batches:</strong> you can upload many files at once; non-.docx files are skipped.</li>
                    <li><strong>Progress and timing:</strong> status/details show parsed files, failed files, parse time, and throughput.</li>
                    <li><strong>What is a Tag:</strong> the card title/label line used to group arguments. In Word, use style <em>Heading 4</em> (also accepts styles named <em>Tag</em>, <em>Tags</em>, <em>Heading 3</em>, or <em>Heading 2</em>).</li>
                    <li><strong>What is a Cite:</strong> the citation/source line (author, date, publication, link). It should be the line right after the tag block and before body paragraphs.</li>
                    <li><strong>What is a Paragraph:</strong> the card body text lines that contain the evidence content. In Word, body paragraphs should use <em>Normal</em> style (also accepts <em>Cards</em>, <em>card</em>, <em>Normal (Web)</em>, or <em>Normal/Card</em>).</li>
                    <li><strong>Word heading setup:</strong> use <em>Heading 4</em> for each card tag, put the citation on the next line, then write evidence paragraphs in <em>Normal</em> style for the most reliable parsing.</li>
                  </ul>
                </section>

                <section className={styles.manualSection}>
                  <h4>3) Query page: search syntax and buttons</h4>
                  <ul>
                    <li><strong>Main Search button:</strong> the top search field runs full-text search when you press Enter or click <em>Search</em>.</li>
                    <li><strong>Citation search syntax:</strong> append <code>cite:your text</code> in the same search box to match citation text. Example: <code>nuclear deterrence cite:brookings</code>.</li>
                    <li><strong>Citation-only queries:</strong> you can use only <code>cite:...</code> (for example <code>cite:harvard law review</code>) to search by citation without tag/paragraph text.</li>
                    <li><strong>URL state:</strong> query and citation filters are stored in URL parameters so refresh/back/forward keep search context.</li>
                    <li><strong>Results tabs:</strong> use <em>Tag Matches</em> and <em>Paragraph Matches</em> to switch match mode views.</li>
                    <li><strong>Pagination buttons:</strong> use <em>Previous</em>, page numbers, and <em>Next</em> to move through result pages.</li>
                    <li><strong>Select a card:</strong> click any result row to load the full card in the detail panel.</li>
                    <li><strong>Query debug console:</strong> bug icon logs search requests/responses and card loading events.</li>
                  </ul>
                </section>

                <section className={styles.manualSection}>
                  <h4>4) Going through cards efficiently</h4>
                  <ul>
                    <li><strong>Read left, edit right:</strong> keep the results list on the left and selected card on the right to move quickly between evidence cards.</li>
                    <li><strong>Card metadata:</strong> each result shows tag preview, citation snippet, and card identifier (CID) when available.</li>
                    <li><strong>Source links:</strong> cards can include one or more source URLs/paths through the download/source area.</li>
                    <li><strong>Copy action:</strong> click <em>Copy</em> to copy formatted card content; a <em>Copied</em> toast confirms success.</li>
                    <li><strong>Edit action:</strong> click <em>Edit</em> on the selected card to enter editing mode.</li>
                    <li><strong>Export Saved Edits:</strong> use the <em>Settings → Export Saved Edits</em> action to export all saved local edits into a single .docx.</li>
                    <li><strong>Style controls:</strong> pick highlight color and font via swatches/dropdown; these preferences affect display and export output.</li>
                  </ul>
                </section>

                <section className={styles.manualSection}>
                  <h4>5) Card editor: how editing works</h4>
                  <p>
                    In edit mode you can directly update tag, tag-sub text, citation, and all card body paragraphs.
                    Selection-based formatting is applied from the editor toolbar.
                  </p>
                  <ul>
                    <li><strong>Highlight:</strong> select text and click the highlighter button (shortcut <strong>F11</strong>).</li>
                    <li><strong>Bold:</strong> select text and click the bold button.</li>
                    <li><strong>Underline:</strong> select text and click underline (shortcut <strong>F9</strong>).</li>
                    <li><strong>Italic:</strong> select text and click italic (shortcut <strong>F5</strong>).</li>
                    <li><strong>Clear formatting:</strong> removes styling from the selected range.</li>
                    <li><strong>Undo/Redo:</strong> use toolbar buttons or keyboard shortcuts <strong>Cmd/Ctrl+Z</strong> and <strong>Cmd/Ctrl+Shift+Z</strong>.</li>
                    <li><strong>Copy while editing:</strong> copy remains available from the top toolbar while in edit mode.</li>
                    <li><strong>Cancel:</strong> exits edit mode and discards unsaved draft changes from that editing session.</li>
                    <li><strong>Save:</strong> writes changes to local saved-edits storage, updates the card view, and exits edit mode.</li>
                  </ul>
                </section>

                <section className={styles.manualSection}>
                  <h4>6) Saved edits and export details</h4>
                  <ul>
                    <li><strong>Where edits live:</strong> saved edits are persisted locally (browser storage), so they remain available between sessions on the same machine/profile.</li>
                    <li><strong>What is exported:</strong> tag, tag-sub, citation, body, and text formatting (highlight/bold/underline/italic) from saved edits.</li>
                    <li><strong>Source labels:</strong> export attempts to include source document labels resolved from card metadata and URLs.</li>
                    <li><strong>Style-aware export:</strong> selected font/highlight settings are applied in generated .docx output.</li>
                  </ul>
                </section>

                <section className={styles.manualSection}>
                  <h4>7) Settings page: complete feature guide</h4>
                  <ul>
                    <li><strong>Appearance / dark mode:</strong> use <em>Switch to Dark Mode</em> or <em>Switch to Light Mode</em>. This updates app surfaces, text contrast, buttons, tabs, and highlight rendering for readability.</li>
                    <li><strong>Clear Parsed Cards:</strong>
                      <ul>
                        <li><strong>Clear parsed cards from index:</strong> removes indexed card data from the search index.</li>
                        <li><strong>Delete uploaded .docx files:</strong> removes document files from the uploaded docs folder.</li>
                        <li><strong>Run Clear Action:</strong> executes whichever clear options are currently checked.</li>
                      </ul>
                    </li>
                    <li><strong>Parser Settings:</strong>
                      <ul>
                        <li><strong>Use parallel processing:</strong> enables parallel parse/index workers.</li>
                        <li><strong>Card workers (cores):</strong> sets worker count for card-level parsing.</li>
                        <li><strong>File workers (cores):</strong> sets worker count for file-level parsing.</li>
                        <li><strong>Enable periodic flush:</strong> toggles periodic persistence/flush during parsing.</li>
                        <li><strong>Flush every N documents:</strong> controls flush interval when periodic flush is enabled.</li>
                        <li><strong>Save Settings:</strong> persists parser settings through the backend API.</li>
                      </ul>
                    </li>
                    <li><strong>Manage Documents popup:</strong>
                      <ul>
                        <li><strong>Search parsed documents:</strong> filter by filename.</li>
                        <li><strong>Show Hidden:</strong> include docs that are not currently in index.</li>
                        <li><strong>Select mode:</strong> enable checkbox selection on rows.</li>
                        <li><strong>Select All:</strong> toggles all currently visible rows in selection mode.</li>
                        <li><strong>Delete Selected:</strong> bulk-deletes selected docs from index/folder where present.</li>
                        <li><strong>Remove from Index:</strong> per-row delete from search index only.</li>
                        <li><strong>Delete File:</strong> per-row delete from uploaded docs folder only.</li>
                      </ul>
                    </li>
                    <li><strong>Manual:</strong> opens this in-app guide.</li>
                    <li><strong>Settings debug console:</strong> logs runtime and settings actions; supports copy/clear/close.</li>
                  </ul>
                </section>

                <section className={styles.manualSection}>
                  <h4>8) End-to-end workflow (recommended)</h4>
                  <ol>
                    <li>Upload and parse .docx files on Home until status confirms parsing completed.</li>
                    <li>Open Query and run search with optional citation filter syntax: <code>cite:...</code>.</li>
                    <li>Switch between <em>Tag Matches</em> and <em>Paragraph Matches</em>, then paginate with <em>Previous/Next</em>.</li>
                    <li>Select cards, review content, and edit the ones you want to keep.</li>
                    <li>Save edits, continue across multiple cards, then export all saved edits from Settings to .docx.</li>
                    <li>Use Settings for theme, parser tuning, and document/index maintenance.</li>
                  </ol>
                </section>

                <section className={styles.manualSection}>
                  <h4>9) Troubleshooting quick tips</h4>
                  <ul>
                    <li>If no results appear, verify documents are parsed/indexed and try broader search text first.</li>
                    <li>If citation filtering seems off, check syntax is exactly <code>cite:your text</code> in the main search box.</li>
                    <li>If actions seem unresponsive, open the page debug console (bug icon) and copy logs for diagnosis.</li>
                    <li>If expected files are missing, check <em>Manage Documents</em> for in-index and in-folder status.</li>
                    <li>If parsing is slow, tune worker counts in <em>Parser Settings</em> based on your CPU capacity.</li>
                  </ul>
                </section>
              </div>
            </div>
          </div>
        )}

        {isDocumentsBoxOpen && (
          <div className={indexStyles['confirm-overlay']} role="presentation" onClick={closeDocumentsBox}>
            <div
              className={indexStyles['documents-dialog']}
              role="dialog"
              aria-modal="true"
              aria-label="Manage documents"
              onClick={(event) => event.stopPropagation()}
            >
              <div className={indexStyles['documents-header']}>
                <h3 className={indexStyles['documents-title']}>Manage Documents</h3>
                <button type="button" className={indexStyles['documents-close']} onClick={closeDocumentsBox} disabled={!!deleteInProgressKey}>Close</button>
              </div>
              <div className={indexStyles['documents-controls']}>
                <input
                  type="text"
                  className={indexStyles['documents-search']}
                  placeholder="Search parsed documents..."
                  value={documentsSearch}
                  onChange={(event) => setDocumentsSearch(event.target.value)}
                />
                <div className={indexStyles['documents-actions-row']}>
                  <label className={indexStyles['documents-toggle']}>
                    <input
                      type="checkbox"
                      checked={showHiddenDocuments}
                      onChange={(event) => setShowHiddenDocuments(event.target.checked)}
                    />
                    Show Hidden
                  </label>
                  <button
                    type="button"
                    className={indexStyles['documents-select-btn']}
                    onClick={onToggleSelectMode}
                    disabled={!!deleteInProgressKey}
                  >
                    {isSelectMode ? 'Exit' : 'Select'}
                  </button>
                  {isSelectMode && (
                    <>
                      <button
                        type="button"
                        className={indexStyles['documents-select-btn']}
                        onClick={onSelectAllVisible}
                        disabled={!!deleteInProgressKey || filteredDocuments.length === 0}
                      >
                        Select All
                      </button>
                      <button
                        type="button"
                        className={indexStyles['document-action-danger']}
                        onClick={onDeleteSelectedDocuments}
                        disabled={!!deleteInProgressKey || selectedDocuments.length === 0}
                      >
                        Delete Selected ({selectedDocuments.length})
                      </button>
                    </>
                  )}
                </div>
              </div>
              {documentsError && <p className={indexStyles['documents-error']}>{documentsError}</p>}
              {isDocumentsLoading && <p className={indexStyles['documents-meta']}>Loading documents...</p>}
              {!isDocumentsLoading && documents.length === 0 && (
                <p className={indexStyles['documents-meta']}>No parsed documents found.</p>
              )}
              {!isDocumentsLoading && documents.length > 0 && filteredDocuments.length === 0 && (
                <p className={indexStyles['documents-meta']}>No documents match your current filters.</p>
              )}
              {!isDocumentsLoading && filteredDocuments.length > 0 && (
                <div className={indexStyles['documents-list']}>
                  {filteredDocuments.map((document) => {
                    const indexKey = `${document.filename}:index`;
                    const folderKey = `${document.filename}:folder`;
                    return (
                      <div key={document.filename} className={indexStyles['document-row']}>
                        {isSelectMode && (
                          <label className={indexStyles['document-select']}>
                            <input
                              type="checkbox"
                              checked={selectedDocuments.includes(document.filename)}
                              onChange={() => toggleSelectedDocument(document.filename)}
                              disabled={!!deleteInProgressKey}
                            />
                          </label>
                        )}
                        <div className={indexStyles['document-main']}>
                          <p className={indexStyles['document-name']}>{document.filename}</p>
                          <p className={indexStyles['document-meta']}>
                            cards: {document.cards_indexed} • in index: {document.in_index ? 'yes' : 'no'} • in folder: {document.in_folder ? 'yes' : 'no'}
                          </p>
                        </div>
                        <div className={indexStyles['document-actions']}>
                          <button
                            type="button"
                            className={indexStyles['document-action-secondary']}
                            disabled={!document.in_index || deleteInProgressKey !== null}
                            onClick={() => onDeleteDocument(document, 'index')}
                          >
                            {deleteInProgressKey === indexKey ? 'Removing…' : 'Remove from Index'}
                          </button>
                          <button
                            type="button"
                            className={indexStyles['document-action-danger']}
                            disabled={!document.in_folder || deleteInProgressKey !== null}
                            onClick={() => onDeleteDocument(document, 'folder')}
                          >
                            {deleteInProgressKey === folderKey ? 'Removing…' : 'Delete File'}
                          </button>
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </>
  );
};

export default SettingsPage;
