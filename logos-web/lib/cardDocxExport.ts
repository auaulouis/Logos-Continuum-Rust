import { Document, Packer, Paragraph, TextRun } from 'docx';
import type { CardEdit } from './cardEdits';
import type { Card } from './types';
import { getCardPreference } from './cardPreferences';

const isBrowser = () => typeof window !== 'undefined';

const safeDecode = (value: string) => {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
};

const getFilenameFromUrl = (url: string): string => {
  if (!url) return '';

  const isLocalPath = url.startsWith('/') && !url.startsWith('http://') && !url.startsWith('https://');
  if (isLocalPath) {
    return safeDecode(url.split('/').pop() || '');
  }

  try {
    const parsedUrl = new URL(url);
    const pathParam = parsedUrl.searchParams.get('path');

    if (pathParam) {
      const decodedPath = safeDecode(pathParam);
      return decodedPath.split('/').pop() || '';
    }

    return safeDecode(parsedUrl.pathname.split('/').pop() || '');
  } catch {
    return '';
  }
};

const normalizeUrlInput = (input?: string | string[]): string[] => {
  if (!input) return [];
  return (Array.isArray(input) ? input : [input]).filter(Boolean);
};

const toUnique = (values: string[]) => values.filter((value, index) => values.indexOf(value) === index);

const sanitizeForFileName = (value: string) => {
  const trimmed = value.trim();
  if (!trimmed) return 'card';
  return trimmed.replace(/[\\/:*?"<>|]/g, '-').replace(/\s+/g, ' ').trim().slice(0, 80) || 'card';
};

const DOCX_HIGHLIGHT_MAP: Record<string, 'yellow' | 'green' | 'cyan'> = {
  yellow: 'yellow',
  lime: 'green',
  aqua: 'cyan',
};

const normalizeSelectedFont = (value?: string) => (value && value.trim() ? value.trim() : 'Calibri');

const normalizeHighlightColor = (value?: string): 'yellow' | 'green' | 'cyan' => {
  return DOCX_HIGHLIGHT_MAP[value || ''] || 'yellow';
};

const readCurrentFontPreference = () => {
  if (!isBrowser()) return 'Calibri';
  return normalizeSelectedFont(window.localStorage.getItem('selectedFont') || undefined);
};

const readCurrentHighlightPreference = () => {
  if (!isBrowser()) return 'yellow';
  return normalizeHighlightColor(window.localStorage.getItem('highlightColor') || undefined);
};

export const extractSourceDocumentLabels = (sourceUrls: string[]): string[] => {
  const labels = sourceUrls
    .map((url) => getFilenameFromUrl(url))
    .filter((name) => name.length > 0);

  return toUnique(labels);
};

const normalizeFileNameLabel = (value?: string) => {
  if (!value) return '';
  const normalized = safeDecode(value.split('/').pop() || value).trim();
  return normalized;
};

const extractCidPrefix = (tag?: string, explicitIdentifier?: string) => {
  const source = [explicitIdentifier, tag].filter(Boolean).join(' ');
  const match = source.match(/CID-(\d{5})-\d{5}/i);
  return match?.[1] || '';
};

const extractCardIdentifier = (tag?: string, explicitIdentifier?: string) => {
  if (explicitIdentifier && explicitIdentifier.trim()) {
    return explicitIdentifier.trim();
  }

  const source = String(tag || '');
  const tokenMatch = source.match(/\[\[(CID-[^\]]+)\]\]/i);
  if (tokenMatch?.[1]) {
    return tokenMatch[1].trim();
  }

  const inlineMatch = source.match(/CID-\d{5}-\d{5}/i);
  if (inlineMatch?.[0]) {
    return inlineMatch[0].trim();
  }

  return '';
};

export const resolveSourceDocumentLabelsFromCard = (input: {
  sourceUrls?: string | string[];
  filename?: string;
}) => {
  const fromUrls = extractSourceDocumentLabels(normalizeUrlInput(input.sourceUrls));
  const fromFilename = normalizeFileNameLabel(input.filename);

  return toUnique([
    ...fromUrls,
    ...(fromFilename ? [fromFilename] : []),
  ]);
};

type FormatRanges = {
  citeEmphasis?: Array<[number, number]>;
  highlights?: Array<[number, number, number]>;
  emphasis?: Array<[number, number, number]>;
  underlines?: Array<[number, number, number]>;
  italics?: Array<[number, number, number]>;
};

const buildInlineRuns = (
  text: string,
  ranges: Array<[number, number]> | undefined,
  options: { bold?: boolean; size?: number; font?: string },
) => {
  const normalized = (ranges || [])
    .map(([start, end]) => clampRange(start, end, text.length))
    .filter((range): range is [number, number] => Array.isArray(range));

  const points = new Set<number>([0, text.length]);
  normalized.forEach(([start, end]) => {
    points.add(start);
    points.add(end);
  });

  const sorted = Array.from(points).sort((a, b) => a - b);
  const runs: TextRun[] = [];

  for (let i = 0; i < sorted.length - 1; i += 1) {
    const start = sorted[i];
    const end = sorted[i + 1];
    if (end <= start) continue;

    const segment = text.slice(start, end);
    if (!segment) continue;

    const isActive = normalized.some(([rangeStart, rangeEnd]) => start >= rangeStart && end <= rangeEnd);
    runs.push(new TextRun({
      text: segment,
      size: options.size || 22,
      bold: options.bold ? isActive : undefined,
      font: normalizeSelectedFont(options.font),
    }));
  }

  if (runs.length === 0) {
    runs.push(new TextRun({ text, size: options.size || 22, font: normalizeSelectedFont(options.font) }));
  }

  return runs;
};

const clampRange = (start: number, end: number, max: number): [number, number] | undefined => {
  const safeStart = Math.max(0, Math.min(start, max));
  const safeEnd = Math.max(safeStart, Math.min(end, max));
  if (safeEnd <= safeStart) return undefined;
  return [safeStart, safeEnd];
};

const rangesForLine = (ranges: Array<[number, number, number]> | undefined, line: number, textLength: number) => {
  return (ranges || [])
    .filter(([rangeLine]) => rangeLine === line)
    .map(([, start, end]) => clampRange(start, end, textLength))
    .filter((range): range is [number, number] => Array.isArray(range));
};

const buildFormattedRuns = (
  text: string,
  line: number,
  formatting: FormatRanges,
  font: string,
  docxHighlightColor: 'yellow' | 'green' | 'cyan',
  defaultSize = 22,
) => {
  const highlights = rangesForLine(formatting.highlights, line, text.length);
  const emphasis = rangesForLine(formatting.emphasis, line, text.length);
  const underlines = rangesForLine(formatting.underlines, line, text.length);
  const italics = rangesForLine(formatting.italics, line, text.length);

  const points = new Set<number>([0, text.length]);
  [...highlights, ...emphasis, ...underlines, ...italics].forEach(([start, end]) => {
    points.add(start);
    points.add(end);
  });

  const sorted = Array.from(points).sort((a, b) => a - b);
  const runs: TextRun[] = [];

  for (let i = 0; i < sorted.length - 1; i += 1) {
    const start = sorted[i];
    const end = sorted[i + 1];
    if (end <= start) continue;

    const segment = text.slice(start, end);
    if (!segment) continue;

    const isHighlighted = highlights.some(([rangeStart, rangeEnd]) => start >= rangeStart && end <= rangeEnd);
    const isBold = emphasis.some(([rangeStart, rangeEnd]) => start >= rangeStart && end <= rangeEnd);
    const isUnderlined = underlines.some(([rangeStart, rangeEnd]) => start >= rangeStart && end <= rangeEnd);
    const isItalic = italics.some(([rangeStart, rangeEnd]) => start >= rangeStart && end <= rangeEnd);

    runs.push(new TextRun({
      text: segment,
      size: defaultSize,
      font: normalizeSelectedFont(font),
      bold: isBold,
      italics: isItalic,
      underline: isUnderlined ? { type: 'single' } : undefined,
      highlight: isHighlighted ? docxHighlightColor : undefined,
    }));
  }

  if (runs.length === 0) {
    runs.push(new TextRun({ text, size: defaultSize, font: normalizeSelectedFont(font) }));
  }

  return runs;
};

const createCardParagraphs = ({
  tag,
  tagSub,
  cite,
  body,
  sourceLabels,
  sourceFallback,
  formatting,
  selectedFont,
  selectedHighlightColor,
  cardIdentifier,
  isStarred,
  customTag,
}: {
  tag: string;
  tagSub?: string;
  cite: string;
  body: string[];
  sourceLabels: string[];
  sourceFallback?: string;
  formatting?: FormatRanges;
  selectedFont?: string;
  selectedHighlightColor?: string;
  cardIdentifier?: string;
  isStarred?: boolean;
  customTag?: string;
}) => {
  const sourceLine = sourceLabels.length > 0
    ? sourceLabels.join(', ')
    : (sourceFallback || 'Unknown source document');

  const effectiveFont = normalizeSelectedFont(selectedFont);
  const effectiveHighlight = normalizeHighlightColor(selectedHighlightColor);
  const normalizedCustomTag = String(customTag || '').trim();
  const normalizedTagSub = String(tagSub || '').trim();
  const appliedTags = toUnique([
    ...(normalizedTagSub ? [normalizedTagSub] : []),
    ...(normalizedCustomTag ? [normalizedCustomTag] : []),
  ]);
  const starredLine = `Starred: ${isStarred ? 'Yes' : 'No'}`;
  const tagsLine = `Tags: ${appliedTags.length > 0 ? appliedTags.join(', ') : 'None'}`;

  const children: Paragraph[] = [
    new Paragraph({
      children: [new TextRun({ text: tag || '', bold: true, size: 32, font: effectiveFont })],
      spacing: { after: 120 },
    }),
  ];

  if (cardIdentifier?.trim()) {
    children.push(new Paragraph({
      children: [new TextRun({
        text: cardIdentifier.trim(),
        size: 16,
        color: 'A6A6A6',
        font: effectiveFont,
      })],
      spacing: { after: 80 },
    }));
  }

  children.push(new Paragraph({
    children: [new TextRun({
      text: starredLine,
      size: 18,
      color: '7A7A7A',
      font: effectiveFont,
    })],
    spacing: { after: 60 },
  }));

  children.push(new Paragraph({
    children: [new TextRun({
      text: tagsLine,
      size: 18,
      color: '7A7A7A',
      font: effectiveFont,
    })],
    spacing: { after: 100 },
  }));

  if (tagSub?.trim()) {
    children.push(new Paragraph({
      children: [new TextRun({ text: tagSub.trim(), size: 22, font: effectiveFont })],
      spacing: { after: 120 },
    }));
  }

  children.push(
    new Paragraph({
      children: [new TextRun({ text: `Source Document: ${sourceLine}`, bold: true, size: 22, font: effectiveFont })],
      spacing: { after: 120 },
    }),
    new Paragraph({
      children: buildInlineRuns(cite || '', formatting?.citeEmphasis, { bold: true, size: 22, font: effectiveFont }),
      spacing: { after: 120 },
    }),
  );

  (body || []).forEach((paragraph, index) => {
    const bodyLine = index + 2;
    children.push(new Paragraph({
      children: buildFormattedRuns(paragraph || '', bodyLine, formatting || {}, effectiveFont, effectiveHighlight, 22),
      spacing: { after: 120 },
    }));
  });

  return children;
};

type ExportCardDocxParams = {
  card: Card;
  sourceUrls?: string | string[];
};

export const exportCardToDocx = async ({ card, sourceUrls }: ExportCardDocxParams) => {
  if (!isBrowser()) return;

  const sourceLabels = resolveSourceDocumentLabelsFromCard({
    sourceUrls,
    filename: card.filename,
  });
  const cidPrefix = extractCidPrefix(card.tag, card.card_identifier);
  const cardPreference = getCardPreference(card.id);
  const sourceFallback = cidPrefix
    ? `Unresolved source (CID prefix ${cidPrefix})`
    : 'Unknown source document';

  const children = createCardParagraphs({
    tag: card.tag,
    tagSub: card.tag_sub,
    cite: card.cite,
    body: card.body || [],
    sourceLabels,
    sourceFallback,
    cardIdentifier: extractCardIdentifier(card.tag, card.card_identifier),
    isStarred: Boolean(cardPreference?.starred),
    customTag: cardPreference?.customTag,
    selectedFont: readCurrentFontPreference(),
    selectedHighlightColor: window.localStorage.getItem('highlightColor') || undefined,
    formatting: {
      citeEmphasis: card.cite_emphasis,
      highlights: card.highlights,
      emphasis: card.emphasis,
      underlines: card.underlines,
      italics: card.italics,
    },
  });

  const document = new Document({
    sections: [{
      children,
    }],
  });

  const blob = await Packer.toBlob(document);
  const url = window.URL.createObjectURL(blob);
  const anchor = window.document.createElement('a');

  const primarySource = sourceLabels[0] || card.id || card.tag || 'card';
  const fileName = `${sanitizeForFileName(primarySource)}-edited-card.docx`;

  anchor.href = url;
  anchor.download = fileName;
  window.document.body.appendChild(anchor);
  anchor.click();
  window.document.body.removeChild(anchor);
  window.URL.revokeObjectURL(url);
};

type SavedCardEditItem = {
  cardId: string;
  edit: CardEdit;
};

export const exportSavedEditsToDocx = async (items: SavedCardEditItem[]) => {
  if (!isBrowser() || items.length === 0) return;

  const cidPrefixToSources: Record<string, string[]> = {};
  items.forEach(({ edit }) => {
    const prefix = extractCidPrefix(edit.tag, edit.cardIdentifier);
    if (!prefix || !edit.sourceDocuments?.length) return;
    const existing = cidPrefixToSources[prefix] || [];
    cidPrefixToSources[prefix] = toUnique([...existing, ...edit.sourceDocuments]);
  });

  const allDocumentLabels = toUnique(items.flatMap(({ edit }) => edit.sourceDocuments || []));
  const unresolvedCidPrefixes = toUnique(items
    .map(({ edit }) => extractCidPrefix(edit.tag, edit.cardIdentifier))
    .filter((prefix) => Boolean(prefix) && !(cidPrefixToSources[prefix] || []).length));

  const children: Paragraph[] = [
    new Paragraph({
      children: [new TextRun({ text: 'Saved Card Edits Export', bold: true, size: 36, font: readCurrentFontPreference() })],
      spacing: { after: 240 },
    }),
  ];

  children.push(new Paragraph({
    children: [new TextRun({ text: 'Documents used in this export:', bold: true, size: 24, font: readCurrentFontPreference() })],
    spacing: { after: 120 },
  }));

  if (allDocumentLabels.length > 0) {
    allDocumentLabels.forEach((label) => {
      children.push(new Paragraph({
        children: [new TextRun({ text: `• ${label}`, size: 22, font: readCurrentFontPreference() })],
        spacing: { after: 60 },
      }));
    });
  } else {
    children.push(new Paragraph({
      children: [new TextRun({ text: '• No document names could be resolved from saved metadata.', size: 22, font: readCurrentFontPreference() })],
      spacing: { after: 60 },
    }));
  }

  if (unresolvedCidPrefixes.length > 0) {
    children.push(new Paragraph({
      children: [new TextRun({ text: 'Unresolved CID prefixes:', bold: true, size: 22, font: readCurrentFontPreference() })],
      spacing: { after: 80 },
    }));
    unresolvedCidPrefixes.forEach((prefix) => {
      children.push(new Paragraph({
        children: [new TextRun({ text: `• ${prefix}`, size: 22, font: readCurrentFontPreference() })],
        spacing: { after: 60 },
      }));
    });
  }

  children.push(new Paragraph({
    children: [new TextRun({ text: ' ', size: 18, font: readCurrentFontPreference() })],
    spacing: { after: 180 },
  }));

  items.forEach(({ cardId, edit }, index) => {
    const cidPrefix = extractCidPrefix(edit.tag, edit.cardIdentifier);
    const cardPreference = getCardPreference(cardId);
    const inferredByCid = cidPrefix ? (cidPrefixToSources[cidPrefix] || []) : [];
    const sourceLabels = (edit.sourceDocuments && edit.sourceDocuments.length > 0)
      ? edit.sourceDocuments
      : inferredByCid;
    const sourceFallback = cidPrefix
      ? `Unresolved source (CID prefix ${cidPrefix})`
      : 'Unknown source document';

    children.push(
      ...createCardParagraphs({
        tag: edit.tag || cardId,
        tagSub: edit.tag_sub,
        cite: edit.cite,
        body: edit.body || [],
        sourceLabels,
        sourceFallback,
        cardIdentifier: extractCardIdentifier(edit.tag, edit.cardIdentifier),
        isStarred: Boolean(cardPreference?.starred),
        customTag: cardPreference?.customTag,
        selectedFont: edit.selectedFont || readCurrentFontPreference(),
        selectedHighlightColor: edit.highlightColor || readCurrentHighlightPreference(),
        formatting: {
          citeEmphasis: edit.citeEmphasis,
          highlights: edit.highlights,
          emphasis: edit.emphasis,
          underlines: edit.underlines,
          italics: edit.italics,
        },
      }),
    );

    if (index < items.length - 1) {
      children.push(new Paragraph({
        children: [new TextRun({ text: ' ', size: 16, font: readCurrentFontPreference() })],
        spacing: { after: 200 },
      }));
    }
  });

  const document = new Document({
    sections: [{
      children,
    }],
  });

  const blob = await Packer.toBlob(document);
  const url = window.URL.createObjectURL(blob);
  const anchor = window.document.createElement('a');

  anchor.href = url;
  anchor.download = `saved-card-edits-${new Date().toISOString().slice(0, 10)}.docx`;
  window.document.body.appendChild(anchor);
  anchor.click();
  window.document.body.removeChild(anchor);
  window.URL.revokeObjectURL(url);
};
