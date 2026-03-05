/* eslint-disable jsx-a11y/control-has-associated-label */
/* eslint-disable @typescript-eslint/no-unused-vars */
/* eslint-disable react/no-danger */
/* eslint-disable react/no-array-index-key */
import {
  forwardRef,
  useRef,
  useContext,
  useEffect,
  useMemo,
  useState,
  useCallback,
  useImperativeHandle,
  type ReactNode,
} from 'react';
import { AppContext } from '../../lib/appContext';
import type { Card } from '../../lib/types';
import { saveCardEdit } from '../../lib/cardEdits';
import { resolveSourceDocumentLabelsFromCard } from '../../lib/cardDocxExport';
import { resolveHighlightColorForTheme } from '../../lib/constants';
import { generateStyledCite, generateStyledParagraph } from '../../lib/utils';
import DownloadLink from '../DownloadLink';
import styles from './styles.module.scss';

type CardProps = {
  card?: Card;
  downloadUrls?: string[];
  onCardSave?: (card: Card) => void;
  externalEditRequest?: number;
  onEditModeChange?: (editing: boolean) => void;
  editorRightActions?: ReactNode;
}

export type CardDetailHandle = {
  copyToClipboard: () => Promise<boolean>;
};

type DraftSnapshot = {
  tagDraft: string;
  tagSubDraft: string;
  citeDraft: string;
  bodyDraft: string[];
  highlightDraft: Array<[number, number, number]>;
  emphasisDraft: Array<[number, number, number]>;
  underlineDraft: Array<[number, number, number]>;
  italicDraft: Array<[number, number, number]>;
};

const LINE_HEIGHT = '107%';

const extractCardIdentifier = (tagValue: string | undefined, explicitIdentifier?: string): string => {
  if (explicitIdentifier && explicitIdentifier.trim()) {
    return explicitIdentifier.trim();
  }

  const tagText = String(tagValue || '');
  const tokenMatch = tagText.match(/\[\[(CID-[^\]]+)\]\]/i);
  if (tokenMatch?.[1]) {
    return tokenMatch[1].trim();
  }

  return '';
};

const stripIdentifierTokenFromTag = (tagValue: string | undefined): string => {
  return String(tagValue || '').replace(/\s*\[\[CID-[^\]]+\]\]\s*/gi, ' ').trim();
};

const extractTagSubHeadline = (value: string | undefined): string => {
  const lines = String(value || '')
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean);
  return lines[0] || '';
};

const CardDetail = forwardRef<CardDetailHandle, CardProps>(({ 
  card,
  downloadUrls,
  onCardSave,
  externalEditRequest = 0,
  onEditModeChange,
  editorRightActions,
}: CardProps, ref) => {
  const styledCite = generateStyledCite(card?.cite, card?.cite_emphasis);
  const container = useRef<HTMLDivElement>(null);
  const headerMetaContainer = useRef<HTMLDivElement>(null);
  const { highlightColor, theme } = useContext(AppContext);
  const [isEditing, setIsEditing] = useState(false);
  const [tagDraft, setTagDraft] = useState('');
  const [tagSubDraft, setTagSubDraft] = useState('');
  const [citeDraft, setCiteDraft] = useState('');
  const [bodyDraft, setBodyDraft] = useState<string[]>([]);
  const [highlightDraft, setHighlightDraft] = useState<Array<[number, number, number]>>([]);
  const [emphasisDraft, setEmphasisDraft] = useState<Array<[number, number, number]>>([]);
  const [underlineDraft, setUnderlineDraft] = useState<Array<[number, number, number]>>([]);
  const [italicDraft, setItalicDraft] = useState<Array<[number, number, number]>>([]);
  const [editMessage, setEditMessage] = useState('');
  const [undoStack, setUndoStack] = useState<DraftSnapshot[]>([]);
  const [redoStack, setRedoStack] = useState<DraftSnapshot[]>([]);
  const [isHeaderOverflowing, setIsHeaderOverflowing] = useState(false);
  const handledExternalEditRequest = useRef(0);
  const cardIdentifier = extractCardIdentifier(card?.tag, card?.card_identifier);
  const displayTag = isEditing
    ? tagDraft
    : (stripIdentifierTokenFromTag(card?.tag) || extractTagSubHeadline(card?.tag_sub));

  const cloneRanges = (ranges: Array<[number, number, number]>) => {
    return ranges.map(([line, start, end]) => [line, start, end] as [number, number, number]);
  };

  const createSnapshot = (): DraftSnapshot => {
    return {
      tagDraft,
      tagSubDraft,
      citeDraft,
      bodyDraft: [...bodyDraft],
      highlightDraft: cloneRanges(highlightDraft),
      emphasisDraft: cloneRanges(emphasisDraft),
      underlineDraft: cloneRanges(underlineDraft),
      italicDraft: cloneRanges(italicDraft),
    };
  };

  const applySnapshot = (snapshot: DraftSnapshot) => {
    setTagDraft(snapshot.tagDraft);
    setTagSubDraft(snapshot.tagSubDraft);
    setCiteDraft(snapshot.citeDraft);
    setBodyDraft([...snapshot.bodyDraft]);
    setHighlightDraft(cloneRanges(snapshot.highlightDraft));
    setEmphasisDraft(cloneRanges(snapshot.emphasisDraft));
    setUnderlineDraft(cloneRanges(snapshot.underlineDraft));
    setItalicDraft(cloneRanges(snapshot.italicDraft));
  };

  const rangesEqual = (a: Array<[number, number, number]>, b: Array<[number, number, number]>) => {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i += 1) {
      if (a[i][0] !== b[i][0] || a[i][1] !== b[i][1] || a[i][2] !== b[i][2]) return false;
    }
    return true;
  };

  const snapshotsEqual = (a: DraftSnapshot, b: DraftSnapshot) => {
    if (a.tagDraft !== b.tagDraft || a.tagSubDraft !== b.tagSubDraft || a.citeDraft !== b.citeDraft) return false;
    if (a.bodyDraft.length !== b.bodyDraft.length) return false;
    for (let i = 0; i < a.bodyDraft.length; i += 1) {
      if (a.bodyDraft[i] !== b.bodyDraft[i]) return false;
    }
    return rangesEqual(a.highlightDraft, b.highlightDraft)
      && rangesEqual(a.emphasisDraft, b.emphasisDraft)
      && rangesEqual(a.underlineDraft, b.underlineDraft)
      && rangesEqual(a.italicDraft, b.italicDraft);
  };

  const applyDraftChange = (updater: (previous: DraftSnapshot) => DraftSnapshot) => {
    const previous = createSnapshot();
    const next = updater(previous);

    if (snapshotsEqual(previous, next)) return false;

    setUndoStack((stack) => [...stack, previous]);
    setRedoStack([]);
    applySnapshot(next);
    return true;
  };

  useEffect(() => {
    if (!card) return;

    setTagDraft(card.tag || '');
    setTagSubDraft(card.tag_sub || '');
    setCiteDraft(card.cite || '');
    setBodyDraft([...(card.body || [])]);
    setHighlightDraft([...(card.highlights || [])]);
    setEmphasisDraft([...(card.emphasis || [])]);
    setUnderlineDraft([...(card.underlines || [])]);
    setItalicDraft([...(card.italics || [])]);
    setEditMessage('');
    setUndoStack([]);
    setRedoStack([]);
    setIsEditing(false);
  }, [card?.id]);

  const draftCard = useMemo(() => {
    if (!card) return undefined;

    return {
      ...card,
      tag: tagDraft,
      tag_sub: tagSubDraft.trim() || undefined,
      cite: citeDraft,
      body: bodyDraft,
      highlights: highlightDraft,
      emphasis: emphasisDraft,
      underlines: underlineDraft,
      italics: italicDraft,
    };
  }, [card, tagDraft, tagSubDraft, citeDraft, bodyDraft, highlightDraft, emphasisDraft, underlineDraft, italicDraft]);

  const effectiveHighlightColor = useMemo(() => {
    return resolveHighlightColorForTheme(highlightColor, theme);
  }, [highlightColor, theme]);

  const copyHighlightColor = useMemo(() => {
    return resolveHighlightColorForTheme(highlightColor, 'light');
  }, [highlightColor]);

  const hasCardChanges = useMemo(() => {
    if (!card) return false;

    const trimmedTagSub = tagSubDraft.trim();
    const currentTagSub = card.tag_sub || '';
    const currentHighlights = card.highlights || [];
    const currentEmphasis = card.emphasis || [];
    const currentUnderlines = card.underlines || [];

    if (tagDraft !== card.tag) return true;
    if (trimmedTagSub !== currentTagSub) return true;
    if (citeDraft !== card.cite) return true;
    if (bodyDraft.length !== card.body.length) return true;

    for (let i = 0; i < bodyDraft.length; i += 1) {
      if (bodyDraft[i] !== card.body[i]) return true;
    }

    if (highlightDraft.length !== currentHighlights.length) return true;
    for (let i = 0; i < highlightDraft.length; i += 1) {
      const next = highlightDraft[i];
      const prev = currentHighlights[i];
      if (!prev || next[0] !== prev[0] || next[1] !== prev[1] || next[2] !== prev[2]) return true;
    }

    if (emphasisDraft.length !== currentEmphasis.length) return true;
    for (let i = 0; i < emphasisDraft.length; i += 1) {
      const next = emphasisDraft[i];
      const prev = currentEmphasis[i];
      if (!prev || next[0] !== prev[0] || next[1] !== prev[1] || next[2] !== prev[2]) return true;
    }

    if (underlineDraft.length !== currentUnderlines.length) return true;
    for (let i = 0; i < underlineDraft.length; i += 1) {
      const next = underlineDraft[i];
      const prev = currentUnderlines[i];
      if (!prev || next[0] !== prev[0] || next[1] !== prev[1] || next[2] !== prev[2]) return true;
    }

    const currentItalics = card.italics || [];
    if (italicDraft.length !== currentItalics.length) return true;
    for (let i = 0; i < italicDraft.length; i += 1) {
      const next = italicDraft[i];
      const prev = currentItalics[i];
      if (!prev || next[0] !== prev[0] || next[1] !== prev[1] || next[2] !== prev[2]) return true;
    }

    return false;
  }, [card, tagDraft, tagSubDraft, citeDraft, bodyDraft, highlightDraft, emphasisDraft, underlineDraft, italicDraft]);

  const escapeHtml = (value: string) => {
    return value
      .replaceAll('&', '&amp;')
      .replaceAll('<', '&lt;')
      .replaceAll('>', '&gt;')
      .replaceAll('"', '&quot;')
      .replaceAll("'", '&#39;');
  };

  const buildCopyHtml = useCallback(() => {
    if (!card) return '';

    const source = draftCard || card;
    const sourceStyledCite = generateStyledCite(source.cite, source.cite_emphasis) || '';
    const tagSub = source.tag_sub?.trim() || '';
    const bodyHtml = source.body
      .map((paragraph, index) => {
        const styledParagraph = generateStyledParagraph(source, index, paragraph, copyHighlightColor);
        return `<p style=\"font-size: 11pt; margin: 0 0 8pt; line-height: ${LINE_HEIGHT}; color: #000000;\">${styledParagraph}</p>`;
      })
      .join('');

    const tagSubHtml = tagSub
      ? `<p style=\"font-size: 11pt; margin: 0 0 8pt; line-height: ${LINE_HEIGHT}; color: #000000;\">${escapeHtml(tagSub)}</p>`
      : '';

    return `<div style=\"color: #000000;\">${
      `<h4 style=\"font-size: 16pt; margin-top: 2px; margin-bottom: 0; line-height: ${LINE_HEIGHT}; color: #000000;\">${escapeHtml(source.tag || '')}</h4>`
      + tagSubHtml
      + `<p style=\"font-size: 11pt; margin-top: 0; margin-bottom: 8px; line-height: ${LINE_HEIGHT}; color: #000000;\">${sourceStyledCite}</p>`
      + bodyHtml
    }</div>`;
  }, [card, draftCard, copyHighlightColor]);

  const htmlToPlainText = (html: string) => {
    const parser = document.createElement('div');
    parser.innerHTML = html;
    return parser.innerText || parser.textContent || '';
  };

  /**
   * Programatically copy the content of the card to the clipboard.
   */
  const copy = useCallback(async () => {
    if (!card) return false;

    const html = buildCopyHtml();
    if (!html) return false;

    const plainText = htmlToPlainText(html);

    if (
      navigator.clipboard
      && typeof navigator.clipboard.write === 'function'
      && typeof window !== 'undefined'
      && 'ClipboardItem' in window
    ) {
      try {
        const ClipboardItemCtor = (window as any).ClipboardItem;
        await navigator.clipboard.write([
          new ClipboardItemCtor({
            'text/html': new Blob([html], { type: 'text/html' }),
            'text/plain': new Blob([plainText], { type: 'text/plain' }),
          }),
        ]);
        return true;
      } catch (error) {
      }
    }

    const temporary = document.createElement('div');
    temporary.setAttribute('contenteditable', 'true');
    temporary.style.position = 'fixed';
    temporary.style.left = '-9999px';
    temporary.style.top = '0';
    temporary.innerHTML = html;
    document.body.appendChild(temporary);

    const selection = window.getSelection();
    const range = document.createRange();
    range.selectNodeContents(temporary);
    selection?.removeAllRanges();
    selection?.addRange(range);

    const copied = document.execCommand('copy');
    selection?.removeAllRanges();
    document.body.removeChild(temporary);
    return copied;
  }, [card, buildCopyHtml]);

  useImperativeHandle(ref, () => ({
    copyToClipboard: copy,
  }), [copy]);

  useEffect(() => {
    if (!card) {
      return;
    }

    if (externalEditRequest > handledExternalEditRequest.current) {
      handledExternalEditRequest.current = externalEditRequest;
      setIsEditing(true);
    }
  }, [externalEditRequest, card?.id]);

  useEffect(() => {
    onEditModeChange?.(isEditing);
  }, [isEditing, onEditModeChange]);

  useEffect(() => {
    const element = headerMetaContainer.current;
    if (!element) {
      setIsHeaderOverflowing(false);
      return;
    }

    const measureOverflow = () => {
      setIsHeaderOverflowing(element.scrollHeight > element.clientHeight + 1);
    };

    measureOverflow();

    if (typeof ResizeObserver === 'undefined') {
      window.addEventListener('resize', measureOverflow);
      return () => {
        window.removeEventListener('resize', measureOverflow);
      };
    }

    const observer = new ResizeObserver(() => {
      measureOverflow();
    });
    observer.observe(element);
    window.addEventListener('resize', measureOverflow);

    return () => {
      observer.disconnect();
      window.removeEventListener('resize', measureOverflow);
    };
  }, [isEditing, displayTag, tagSubDraft, citeDraft, card?.tag_sub, styledCite]);

  const onBodyParagraphChange = (paragraphIndex: number, value: string) => {
    const line = paragraphIndex + 2;

    applyDraftChange((previous) => {
      const clampRanges = (ranges: Array<[number, number, number]>) => {
        return ranges
          .filter((item) => item[0] !== line || item[1] < value.length)
          .map((item) => {
            if (item[0] !== line) return item;
            return [item[0], item[1], Math.min(item[2], value.length)] as [number, number, number];
          })
          .filter((item) => item[2] > item[1]);
      };

      const nextBody = [...previous.bodyDraft];
      nextBody[paragraphIndex] = value;

      return {
        ...previous,
        bodyDraft: nextBody,
        highlightDraft: clampRanges(previous.highlightDraft),
        emphasisDraft: clampRanges(previous.emphasisDraft),
        underlineDraft: clampRanges(previous.underlineDraft),
        italicDraft: clampRanges(previous.italicDraft),
      };
    });
  };

  const findParagraphElement = (node: Node | null): HTMLElement | null => {
    if (!node) return null;
    const element = node.nodeType === Node.ELEMENT_NODE ? node as HTMLElement : node.parentElement;
    return element?.closest('[data-paragraph-index]') as HTMLElement | null;
  };

  const getOffsetInElement = (root: HTMLElement, targetNode: Node, targetOffset: number): number => {
    let offset = 0;
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    let current = walker.nextNode();

    while (current) {
      if (current === targetNode) {
        return offset + targetOffset;
      }
      offset += current.textContent?.length || 0;
      current = walker.nextNode();
    }

    return offset;
  };

  const getCurrentSelectionRange = () => {
    const selection = window.getSelection();
    if (!selection || selection.rangeCount === 0 || selection.isCollapsed) {
      return undefined;
    }

    const range = selection.getRangeAt(0);
    const startParagraph = findParagraphElement(range.startContainer);
    const endParagraph = findParagraphElement(range.endContainer);

    if (!startParagraph || !endParagraph || startParagraph !== endParagraph) {
      return undefined;
    }

    const paragraphIndex = Number(startParagraph.dataset.paragraphIndex || '-1');
    if (Number.isNaN(paragraphIndex) || paragraphIndex < 0) {
      return undefined;
    }

    const start = getOffsetInElement(startParagraph, range.startContainer, range.startOffset);
    const end = getOffsetInElement(startParagraph, range.endContainer, range.endOffset);
    const safeStart = Math.min(start, end);
    const safeEnd = Math.max(start, end);

    if (safeStart === safeEnd) {
      return undefined;
    }

    return {
      selection,
      line: paragraphIndex + 2,
      safeStart,
      safeEnd,
    };
  };

  const clearRangesInSelection = (
    ranges: Array<[number, number, number]>,
    line: number,
    start: number,
    end: number,
  ) => {
    const next: Array<[number, number, number]> = [];

    for (const [rangeLine, rangeStart, rangeEnd] of ranges) {
      if (rangeLine !== line || end <= rangeStart || start >= rangeEnd) {
        next.push([rangeLine, rangeStart, rangeEnd]);
        continue;
      }

      if (start > rangeStart) {
        next.push([rangeLine, rangeStart, start]);
      }

      if (end < rangeEnd) {
        next.push([rangeLine, end, rangeEnd]);
      }
    }

    next.sort((a, b) => (a[0] - b[0]) || (a[1] - b[1]));
    return next;
  };

  const isSelectionFullyFormatted = (
    ranges: Array<[number, number, number]>,
    line: number,
    start: number,
    end: number,
  ) => {
    const lineRanges = ranges
      .filter(([rangeLine, rangeStart, rangeEnd]) => {
        return rangeLine === line && rangeEnd > start && rangeStart < end;
      })
      .map(([, rangeStart, rangeEnd]) => [Math.max(start, rangeStart), Math.min(end, rangeEnd)] as [number, number]);

    if (!lineRanges.length) return false;

    lineRanges.sort((a, b) => a[0] - b[0]);
    let coveredUntil = start;

    for (const [rangeStart, rangeEnd] of lineRanges) {
      if (rangeStart > coveredUntil) {
        return false;
      }
      coveredUntil = Math.max(coveredUntil, rangeEnd);
      if (coveredUntil >= end) {
        return true;
      }
    }

    return coveredUntil >= end;
  };

  const toggleSelectionRanges = (
    ranges: Array<[number, number, number]>,
    line: number,
    start: number,
    end: number,
  ) => {
    if (isSelectionFullyFormatted(ranges, line, start, end)) {
      return {
        next: clearRangesInSelection(ranges, line, start, end),
        removed: true,
      };
    }

    const next: Array<[number, number, number]> = [
      ...ranges,
      [line, start, end],
    ];
    next.sort((a, b) => (a[0] - b[0]) || (a[1] - b[1]));

    return {
      next,
      removed: false,
    };
  };

  const highlightSelection = () => {
    const selected = getCurrentSelectionRange();
    if (!selected) {
      setEditMessage('Select text in one paragraph first.');
      return;
    }

    const { removed } = toggleSelectionRanges(
      highlightDraft,
      selected.line,
      selected.safeStart,
      selected.safeEnd,
    );

    applyDraftChange((previous) => {
      const toggled = toggleSelectionRanges(
        previous.highlightDraft,
        selected.line,
        selected.safeStart,
        selected.safeEnd,
      );
      return {
        ...previous,
        highlightDraft: toggled.next,
      };
    });

    selected.selection.removeAllRanges();
    setEditMessage(removed ? 'Highlight removed.' : 'Highlight added.');
  };

  const boldSelection = () => {
    const selected = getCurrentSelectionRange();
    if (!selected) {
      setEditMessage('Select text in one paragraph first.');
      return;
    }

    const { removed } = toggleSelectionRanges(
      emphasisDraft,
      selected.line,
      selected.safeStart,
      selected.safeEnd,
    );

    applyDraftChange((previous) => {
      const toggled = toggleSelectionRanges(
        previous.emphasisDraft,
        selected.line,
        selected.safeStart,
        selected.safeEnd,
      );
      return {
        ...previous,
        emphasisDraft: toggled.next,
      };
    });

    selected.selection.removeAllRanges();
    setEditMessage(removed ? 'Bold removed.' : 'Bold added.');
  };

  const underlineSelection = () => {
    const selected = getCurrentSelectionRange();
    if (!selected) {
      setEditMessage('Select text in one paragraph first.');
      return;
    }

    const { removed } = toggleSelectionRanges(
      underlineDraft,
      selected.line,
      selected.safeStart,
      selected.safeEnd,
    );

    applyDraftChange((previous) => {
      const toggled = toggleSelectionRanges(
        previous.underlineDraft,
        selected.line,
        selected.safeStart,
        selected.safeEnd,
      );
      return {
        ...previous,
        underlineDraft: toggled.next,
      };
    });

    selected.selection.removeAllRanges();
    setEditMessage(removed ? 'Underline removed.' : 'Underline added.');
  };

  const italicSelection = () => {
    const selected = getCurrentSelectionRange();
    if (!selected) {
      setEditMessage('Select text in one paragraph first.');
      return;
    }

    const { removed } = toggleSelectionRanges(
      italicDraft,
      selected.line,
      selected.safeStart,
      selected.safeEnd,
    );

    applyDraftChange((previous) => {
      const toggled = toggleSelectionRanges(
        previous.italicDraft,
        selected.line,
        selected.safeStart,
        selected.safeEnd,
      );
      return {
        ...previous,
        italicDraft: toggled.next,
      };
    });

    selected.selection.removeAllRanges();
    setEditMessage(removed ? 'Italics removed.' : 'Italics added.');
  };

  const clearSelectionFormatting = () => {
    const selected = getCurrentSelectionRange();
    if (!selected) {
      setEditMessage('Select text in one paragraph first.');
      return;
    }

    const changed = applyDraftChange((previous) => {
      return {
        ...previous,
        highlightDraft: clearRangesInSelection(previous.highlightDraft, selected.line, selected.safeStart, selected.safeEnd),
        emphasisDraft: clearRangesInSelection(previous.emphasisDraft, selected.line, selected.safeStart, selected.safeEnd),
        underlineDraft: clearRangesInSelection(previous.underlineDraft, selected.line, selected.safeStart, selected.safeEnd),
        italicDraft: clearRangesInSelection(previous.italicDraft, selected.line, selected.safeStart, selected.safeEnd),
      };
    });

    selected.selection.removeAllRanges();
    setEditMessage(changed ? 'Formatting cleared.' : 'No formatting found in selection.');
  };

  const onCancel = () => {
    if (!card) return;

    setTagDraft(card.tag || '');
    setTagSubDraft(card.tag_sub || '');
    setCiteDraft(card.cite || '');
    setBodyDraft([...(card.body || [])]);
    setHighlightDraft([...(card.highlights || [])]);
    setEmphasisDraft([...(card.emphasis || [])]);
    setUnderlineDraft([...(card.underlines || [])]);
    setItalicDraft([...(card.italics || [])]);
    setEditMessage('');
    setUndoStack([]);
    setRedoStack([]);
    setIsEditing(false);
  };

  const onUndo = () => {
    if (!undoStack.length) return;

    const previous = undoStack[undoStack.length - 1];
    const current = createSnapshot();

    setUndoStack((stack) => stack.slice(0, -1));
    setRedoStack((stack) => [...stack, current]);
    applySnapshot(previous);
    setEditMessage('Undo applied.');
  };

  const onRedo = () => {
    if (!redoStack.length) return;

    const next = redoStack[redoStack.length - 1];
    const current = createSnapshot();

    setRedoStack((stack) => stack.slice(0, -1));
    setUndoStack((stack) => [...stack, current]);
    applySnapshot(next);
    setEditMessage('Redo applied.');
  };

  useEffect(() => {
    if (!isEditing) return undefined;

    const onKeyDown = (event: KeyboardEvent) => {
      const key = event.key;
      const isModifierUndo = (event.ctrlKey || event.metaKey) && !event.altKey && key.toLowerCase() === 'z';

      if (isModifierUndo) {
        event.preventDefault();
        if (event.shiftKey) {
          onRedo();
        } else {
          onUndo();
        }
        return;
      }

      if (event.ctrlKey || event.metaKey || event.altKey) return;

      if (key === 'F11') {
        event.preventDefault();
        highlightSelection();
        return;
      }

      if (key === 'F9') {
        event.preventDefault();
        underlineSelection();
        return;
      }

      if (key === 'F5') {
        event.preventDefault();
        italicSelection();
      }
    };

    window.addEventListener('keydown', onKeyDown);
    return () => {
      window.removeEventListener('keydown', onKeyDown);
    };
  }, [isEditing, onUndo, onRedo, highlightSelection, underlineSelection, italicSelection]);

  const onSave = () => {
    if (!card) return;

    const highlights: Array<[number, number, number]> = highlightDraft
      .map(([line, start, end]) => {
        const paragraphIndex = line - 2;
        const paragraph = bodyDraft[paragraphIndex] || '';
        const safeStart = Math.max(0, Math.min(start, paragraph.length));
        const safeEnd = Math.max(safeStart, Math.min(end, paragraph.length));
        return [line, safeStart, safeEnd] as [number, number, number];
      })
      .filter((item) => item[2] > item[1]);

    const emphases: Array<[number, number, number]> = emphasisDraft
      .map(([line, start, end]) => {
        const paragraphIndex = line - 2;
        const paragraph = bodyDraft[paragraphIndex] || '';
        const safeStart = Math.max(0, Math.min(start, paragraph.length));
        const safeEnd = Math.max(safeStart, Math.min(end, paragraph.length));
        return [line, safeStart, safeEnd] as [number, number, number];
      })
      .filter((item) => item[2] > item[1]);

    const underlines: Array<[number, number, number]> = underlineDraft
      .map(([line, start, end]) => {
        const paragraphIndex = line - 2;
        const paragraph = bodyDraft[paragraphIndex] || '';
        const safeStart = Math.max(0, Math.min(start, paragraph.length));
        const safeEnd = Math.max(safeStart, Math.min(end, paragraph.length));
        return [line, safeStart, safeEnd] as [number, number, number];
      })
      .filter((item) => item[2] > item[1]);

    const italics: Array<[number, number, number]> = italicDraft
      .map(([line, start, end]) => {
        const paragraphIndex = line - 2;
        const paragraph = bodyDraft[paragraphIndex] || '';
        const safeStart = Math.max(0, Math.min(start, paragraph.length));
        const safeEnd = Math.max(safeStart, Math.min(end, paragraph.length));
        return [line, safeStart, safeEnd] as [number, number, number];
      })
      .filter((item) => item[2] > item[1]);

    const updatedCard: Card = {
      ...card,
      tag: tagDraft,
      tag_sub: tagSubDraft.trim() || undefined,
      cite: citeDraft,
      body: bodyDraft,
      highlights,
      emphasis: emphases,
      underlines,
      italics,
    };

    const sourceLabels = resolveSourceDocumentLabelsFromCard({
      sourceUrls: downloadUrls || card.download_url || card.s3_url,
      filename: card.filename,
    });
    const selectedFont = typeof window !== 'undefined'
      ? window.localStorage.getItem('selectedFont') || undefined
      : undefined;

    try {
      saveCardEdit(updatedCard.id, {
        tag: updatedCard.tag,
        tag_sub: updatedCard.tag_sub,
        cite: updatedCard.cite,
        citeEmphasis: updatedCard.cite_emphasis || [],
        body: updatedCard.body,
        highlights: updatedCard.highlights,
        emphasis: updatedCard.emphasis,
        underlines: updatedCard.underlines,
        italics: updatedCard.italics || [],
        sourceDocuments: sourceLabels,
        cardIdentifier: card.card_identifier,
        selectedFont,
        highlightColor,
      });

      onCardSave?.(updatedCard);
      setUndoStack([]);
      setRedoStack([]);
    } finally {
      setIsEditing(false);
    }
  };

  const editorToolbar = isEditing ? (
    <div className={`${styles['editor-toolbar']} ${isHeaderOverflowing ? styles['editor-toolbar-overflow'] : ''}`}>
      <button
        className={`${styles['toolbar-action']} ${styles['editor-toolbar-action']}`}
        type="button"
        onClick={highlightSelection}
        aria-label="Highlight selection"
        title="Highlight selection"
      >
        <img
          src="/stylus_highlighter_24dp_E3E3E3_FILL0_wght400_GRAD0_opsz24.png"
          alt="Highlight selection"
          className={styles['icon-image']}
        />
      </button>
      <button
        className={`${styles['toolbar-action']} ${styles['editor-toolbar-action']}`}
        type="button"
        onClick={boldSelection}
        aria-label="Bold selection"
        title="Bold selection"
      >
        <img
          src="/format_bold_24dp_E3E3E3_FILL0_wght400_GRAD0_opsz24.png"
          alt="Bold selection"
          className={styles['icon-image']}
        />
      </button>
      <button
        className={`${styles['toolbar-action']} ${styles['editor-toolbar-action']}`}
        type="button"
        onClick={underlineSelection}
        aria-label="Underline selection"
        title="Underline selection"
      >
        <img
          src="/format_underlined_24dp_E3E3E3_FILL0_wght400_GRAD0_opsz24.png"
          alt="Underline selection"
          className={styles['icon-image']}
        />
      </button>
      <button
        className={`${styles['toolbar-action']} ${styles['editor-toolbar-action']}`}
        type="button"
        onClick={italicSelection}
        aria-label="Italicize selection"
        title="Italicize selection"
      >
        <img
          src="/format_italic_24dp_E3E3E3_FILL0_wght400_GRAD0_opsz24.png"
          alt="Italicize selection"
          className={styles['icon-image']}
        />
      </button>
      <button
        className={`${styles['toolbar-action']} ${styles['editor-toolbar-action']}`}
        type="button"
        onClick={clearSelectionFormatting}
        aria-label="Clear selected formatting"
        title="Clear selected formatting"
      >
        <img
          src="/format_clear_24dp_E3E3E3_FILL0_wght400_GRAD0_opsz24.png"
          alt="Clear selected formatting"
          className={styles['icon-image']}
        />
      </button>
      <button
        className={`${styles['toolbar-action']} ${styles['editor-toolbar-action']}`}
        type="button"
        onClick={onUndo}
        disabled={!undoStack.length}
        aria-label="Undo"
        title="Undo"
      >
        <img
          src="/undo_24dp_E3E3E3_FILL0_wght400_GRAD0_opsz24.png"
          alt="Undo"
          className={styles['icon-image']}
        />
      </button>
      <button
        className={`${styles['toolbar-action']} ${styles['editor-toolbar-action']}`}
        type="button"
        onClick={onRedo}
        disabled={!redoStack.length}
        aria-label="Redo"
        title="Redo"
      >
        <img
          src="/redo_24dp_E3E3E3_FILL0_wght400_GRAD0_opsz24.png"
          alt="Redo"
          className={styles['icon-image']}
        />
      </button>
      <button
        className={`${styles['toolbar-action']} ${styles['editor-toolbar-action']}`}
        type="button"
        onClick={onCancel}
        aria-label="Cancel"
        title="Cancel"
      >
        <img
          src="/cancel_24dp_E3E3E3_FILL0_wght400_GRAD0_opsz24.png"
          alt="Cancel"
          className={styles['icon-image']}
        />
      </button>
      <button
        className={`${styles['toolbar-action']} ${styles['editor-toolbar-action']}`}
        type="button"
        onClick={onSave}
        disabled={!hasCardChanges}
        aria-label="Save"
        title="Save"
      >
        <img
          src="/save_24dp_E3E3E3_FILL0_wght400_GRAD0_opsz24.png"
          alt="Save"
          className={styles['icon-image']}
        />
      </button>
      {!!editorRightActions && (
        <div className={styles['editor-toolbar-right']}>
          {editorRightActions}
        </div>
      )}
    </div>
  ) : null;

  return (
    <div className={styles.card}>
      {!!card && (
        <>
          <div ref={container} className={styles['card-content']}>
            <div
              ref={headerMetaContainer}
              className={`${styles['card-header']} ${styles['card-header-scrollable']} ${isHeaderOverflowing ? styles['card-header-overflowing'] : ''}`}
            >
              {isEditing && isHeaderOverflowing && editorToolbar}
              <div className={styles['copy-container']}>
                <h4
                  className={`${isEditing ? styles['editable-block'] : ''} ${styles['card-tag-title']}`}
                  style={{
                    fontSize: '16pt', marginTop: 2, marginBottom: 0, lineHeight: LINE_HEIGHT,
                  }}
                  contentEditable={isEditing}
                  suppressContentEditableWarning
                  onBlur={(e) => {
                    const nextValue = e.currentTarget.textContent || '';
                    applyDraftChange((previous) => ({
                      ...previous,
                      tagDraft: nextValue,
                    }));
                  }}
                >{displayTag}
                </h4>
                {!isEditing && !!cardIdentifier && <div className={styles['card-cid']}>{cardIdentifier}</div>}
              </div>
              {isEditing && !isHeaderOverflowing && editorToolbar}
              {(isEditing || !!card.tag_sub) && (
                <p
                  className={`MsoNormal ${isEditing ? styles['editable-block'] : ''} ${styles['card-tag-sub']}`}
                  style={{ fontSize: '11pt', margin: '0in 0in 8pt', lineHeight: LINE_HEIGHT }}
                  contentEditable={isEditing}
                  suppressContentEditableWarning
                  onBlur={(e) => {
                    const nextValue = e.currentTarget.textContent || '';
                    applyDraftChange((previous) => ({
                      ...previous,
                      tagSubDraft: nextValue,
                    }));
                  }}
                >{isEditing ? tagSubDraft : card.tag_sub}
                </p>
              )}

              <p
                className={`MsoNormal ${isEditing ? styles['editable-block'] : ''} ${styles['card-cite']}`}
                style={{
                  fontSize: '11pt', marginTop: 0, marginBottom: 8, lineHeight: LINE_HEIGHT,
                }}
                contentEditable={isEditing}
                suppressContentEditableWarning
                onBlur={(e) => {
                  const nextValue = e.currentTarget.textContent || '';
                  applyDraftChange((previous) => ({
                    ...previous,
                    citeDraft: nextValue,
                  }));
                }}
                dangerouslySetInnerHTML={{ __html: isEditing ? citeDraft : styledCite || '' }}
              />
            </div>

            <div className={styles['paragraphs-scroll']}>
              {(isEditing ? bodyDraft : card.body).map((paragraph, i) => {
                const styledParagraph = draftCard ? generateStyledParagraph(draftCard, i, paragraph, effectiveHighlightColor) : paragraph;

                return (
                  <p
                    className={`MsoNormal ${isEditing ? styles['editable-block'] : ''}`}
                    style={{ fontSize: '11pt', margin: '0in 0in 8pt', lineHeight: LINE_HEIGHT }}
                    key={i}
                    data-paragraph-index={i}
                    contentEditable={isEditing}
                    suppressContentEditableWarning
                    onBlur={(e) => onBodyParagraphChange(i, e.currentTarget.textContent || '')}
                    dangerouslySetInnerHTML={{ __html: styledParagraph }}
                  />
                );
              })}
              {isEditing && !!editMessage && <div className={styles['editor-message']}>{editMessage}</div>}
            </div>
          </div>
          <div className={styles.download}>
            <DownloadLink url={downloadUrls || card.download_url || card.s3_url} />
          </div>
        </>
      )}
    </div>
  );
});

CardDetail.displayName = 'CardDetail';

export default CardDetail;
