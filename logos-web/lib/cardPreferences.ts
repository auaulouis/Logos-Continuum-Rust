import type { Card } from './types';

const STORAGE_KEY = 'logos-card-preferences-v1';
const UPDATE_EVENT = 'logos-card-preferences-updated';

export type CardSnapshot = {
  id: string;
  tag: string;
  cite: string;
  card_identifier?: string;
};

export type CardPreference = {
  starred: boolean;
  customTag?: string;
  updatedAt: number;
  snapshot?: CardSnapshot;
};

type CustomTagOptions = {
  starredOnly?: boolean;
};

const isBrowser = () => typeof window !== 'undefined';

const emitUpdate = () => {
  if (!isBrowser()) return;
  window.dispatchEvent(new Event(UPDATE_EVENT));
};

const readAllPreferences = (): Record<string, CardPreference> => {
  if (!isBrowser()) return {};

  const raw = window.localStorage.getItem(STORAGE_KEY);
  if (!raw) return {};

  try {
    const parsed = JSON.parse(raw) as Record<string, CardPreference>;
    return parsed && typeof parsed === 'object' ? parsed : {};
  } catch {
    return {};
  }
};

const writeAllPreferences = (preferences: Record<string, CardPreference>) => {
  if (!isBrowser()) return;
  window.localStorage.setItem(STORAGE_KEY, JSON.stringify(preferences));
  emitUpdate();
};

const buildSnapshotFromCard = (card: Card): CardSnapshot => ({
  id: card.id,
  tag: card.tag,
  cite: card.cite,
  card_identifier: card.card_identifier,
});

const normalizeCustomTag = (value?: string): string | undefined => {
  const next = String(value || '').trim();
  return next.length > 0 ? next : undefined;
};

const setOrDeletePreference = (
  preferences: Record<string, CardPreference>,
  cardId: string,
  next: CardPreference,
) => {
  if (!next.starred && !next.customTag) {
    const nextPreferences = { ...preferences };
    delete nextPreferences[cardId];
    return nextPreferences;
  }

  return {
    ...preferences,
    [cardId]: next,
  };
};

export const getCardPreference = (cardId: string): CardPreference | undefined => {
  return readAllPreferences()[cardId];
};

export const setCardStarred = (card: Card, starred: boolean) => {
  const preferences = readAllPreferences();
  const previous = preferences[card.id];
  const next: CardPreference = {
    starred,
    customTag: normalizeCustomTag(previous?.customTag),
    updatedAt: Date.now(),
    snapshot: buildSnapshotFromCard(card),
  };

  const nextPreferences = setOrDeletePreference(preferences, card.id, next);
  writeAllPreferences(nextPreferences);
  return next;
};

export const setCardCustomTag = (card: Card, customTag?: string) => {
  const preferences = readAllPreferences();
  const previous = preferences[card.id];
  const nextTag = normalizeCustomTag(customTag);

  const next: CardPreference = {
    starred: Boolean(previous?.starred),
    customTag: nextTag,
    updatedAt: Date.now(),
    snapshot: buildSnapshotFromCard(card),
  };

  const nextPreferences = setOrDeletePreference(preferences, card.id, next);
  writeAllPreferences(nextPreferences);
  return next;
};

export const updateCardPreferenceSnapshot = (card: Card) => {
  const preferences = readAllPreferences();
  const previous = preferences[card.id];
  if (!previous) return;

  preferences[card.id] = {
    ...previous,
    snapshot: buildSnapshotFromCard(card),
    updatedAt: previous.updatedAt,
  };

  writeAllPreferences(preferences);
};

export const getAllCardPreferences = (): Record<string, CardPreference> => {
  return readAllPreferences();
};

export const getAllCustomTags = (options?: CustomTagOptions): string[] => {
  const tagSet = new Set<string>();
  const starredOnly = Boolean(options?.starredOnly);

  Object.values(readAllPreferences()).forEach((entry) => {
    if (starredOnly && !entry.starred) {
      return;
    }

    const normalized = normalizeCustomTag(entry.customTag);
    if (normalized) {
      tagSet.add(normalized);
    }
  });

  return Array.from(tagSet).sort((a, b) => a.localeCompare(b));
};

export const getStarredCardIds = (): string[] => {
  return Object.entries(readAllPreferences())
    .filter(([, entry]) => Boolean(entry.starred))
    .sort((a, b) => (b[1].updatedAt || 0) - (a[1].updatedAt || 0))
    .map(([cardId]) => cardId);
};

export const subscribeToCardPreferences = (listener: () => void): (() => void) => {
  if (!isBrowser()) {
    return () => undefined;
  }

  window.addEventListener(UPDATE_EVENT, listener);
  return () => {
    window.removeEventListener(UPDATE_EVENT, listener);
  };
};
