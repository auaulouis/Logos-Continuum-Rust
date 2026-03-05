import axios from 'axios';
import {
  ensureDesktopBackendRunning,
  invokeTauriCommand,
  isDesktopRuntime,
  isTauriRuntime,
} from '../lib/desktopRuntime';

const oldUrl = 'https://logos-web.onrender.com';
const newUrl = 'https://logos-debate.duckdns.org';
const desktopApiUrl = 'http://127.0.0.1:5501';

const getApiUrl = () => {
  if (isDesktopRuntime()) {
    return desktopApiUrl;
  }

  if (process.env.NEXT_PUBLIC_API_URL) {
    return process.env.NEXT_PUBLIC_API_URL;
  }

  return process.env.NODE_ENV === 'development' ? 'http://localhost:5002' : newUrl;
};

type SearchOptions = {
  signal?: AbortSignal;
};

export const search = async (
  query: string,
  cursor = 0,
  additionalParams = {},
  limit = 30,
  options: SearchOptions = {},
) => {
  if (isTauriRuntime()) {
    await ensureDesktopBackendRunning();
    const response = await invokeTauriCommand<{
      results: Array<Record<string, unknown>>;
      cursor: number;
      total_count: number;
      has_more: boolean;
      count_is_partial: boolean;
    }>('query_cards', {
      params: {
        search: query,
        cursor,
        limit,
        ...additionalParams,
      },
    });

    return {
      results: response.results,
      cursor: response.cursor,
      totalCount: response.total_count,
      hasMore: Boolean(response.has_more),
      countIsPartial: Boolean(response.count_is_partial),
    };
  }

  const apiUrl = getApiUrl();
  let url = `${apiUrl}/query?search=${encodeURIComponent(query)}&cursor=${cursor}&limit=${limit}`;
  Object.entries(additionalParams).forEach(([key, value]) => {
    url += `&${encodeURIComponent(key)}=${encodeURIComponent(String(value))}`;
  });

  const response = await axios.get(url, { signal: options.signal });
  return {
    results: response.data.results,
    cursor: response.data.cursor,
    totalCount: response.data.total_count,
    hasMore: Boolean(response.data.has_more),
    countIsPartial: Boolean(response.data.count_is_partial),
  };
};

export const getCard = async (id: string) => {
  if (isTauriRuntime()) {
    await ensureDesktopBackendRunning();
    return invokeTauriCommand('get_card', { id });
  }

  const apiUrl = getApiUrl();
  const response = await axios.get(`${apiUrl}/card?id=${id}`);
  return response.data;
};

export const getSchools = async () => {
  if (isTauriRuntime()) {
    await ensureDesktopBackendRunning();
    return invokeTauriCommand('get_schools');
  }

  const apiUrl = getApiUrl();
  const response = await axios.get(`${apiUrl}/schools`);
  return response.data;
};

export const uploadDocx = async (file: File, options?: { parseImmediately?: boolean }) => {
  if (isTauriRuntime()) {
    await ensureDesktopBackendRunning();
    const buffer = await file.arrayBuffer();
    const bytes = Array.from(new Uint8Array(buffer));
    return invokeTauriCommand<{
      ok: boolean;
      queued?: boolean;
      deferred?: boolean;
      filename: string;
      cards_indexed: number;
      parse_ms?: number;
    }>('upload_docx', {
      filename: file.name,
      bytes,
      parseImmediately: options?.parseImmediately,
    });
  }

  const apiUrl = getApiUrl();
  const formData = new FormData();
  formData.append('file', file);
  if (options?.parseImmediately === false) {
    formData.append('parse', 'false');
  }
  const response = await axios.post(`${apiUrl}/upload-docx`, formData, {
    headers: { 'Content-Type': 'multipart/form-data' },
  });
  return response.data as {
    ok: boolean;
    queued?: boolean;
    deferred?: boolean;
    filename: string;
    cards_indexed: number;
    parse_ms?: number;
  };
};

export const parseUploadedDocs = async () => {
  if (isTauriRuntime()) {
    await ensureDesktopBackendRunning();
    return invokeTauriCommand<{
      ok: boolean;
      queued: number;
      skipped_already_indexed: number;
    }>('parse_uploaded_docs');
  }

  const apiUrl = getApiUrl();
  const response = await axios.post(`${apiUrl}/parse-uploaded-docs`);
  return response.data as {
    ok: boolean;
    queued: number;
    skipped_already_indexed: number;
  };
};

export const clearIndex = async () => {
  if (isTauriRuntime()) {
    await ensureDesktopBackendRunning();
    return invokeTauriCommand<{ ok: boolean }>('clear_index');
  }

  const apiUrl = getApiUrl();
  const response = await axios.post(`${apiUrl}/clear-index`);
  return response.data as { ok: boolean };
};

export type ParsedDocument = {
  filename: string;
  cards_indexed: number;
  in_index: boolean;
  in_folder: boolean;
  folder_path?: string | null;
};

export type ParserSettings = {
  use_parallel_processing: boolean;
  parser_card_workers: number;
  local_parser_file_workers: number;
  flush_enabled: boolean;
  flush_every_docs: number;
};

export type ParserEvent = {
  id: string;
  at: number;
  level: 'info' | 'warn' | 'error';
  message: string;
  source?: string;
  filename?: string;
  cards_indexed?: number;
  parse_ms?: number;
  cards_per_second?: number;
};

export const getParsedDocuments = async () => {
  if (isTauriRuntime()) {
    await ensureDesktopBackendRunning();
    return invokeTauriCommand<{ documents: ParsedDocument[] }>('get_documents');
  }

  const apiUrl = getApiUrl();
  const response = await axios.get(`${apiUrl}/documents`);
  return response.data as { documents: ParsedDocument[] };
};

export const deleteParsedDocument = async (filename: string, target: 'index' | 'folder') => {
  if (isTauriRuntime()) {
    await ensureDesktopBackendRunning();
    return invokeTauriCommand<{
      ok: boolean;
      removed_cards: number;
      removed_from_folder: boolean;
      deleted_path: string | null;
      message?: string;
    }>('delete_document', { filename, target });
  }

  const apiUrl = getApiUrl();
  const response = await axios.post(`${apiUrl}/delete-document`, { filename, target });
  return response.data as {
    ok: boolean;
    removed_cards: number;
    removed_from_folder: boolean;
    deleted_path: string | null;
    message?: string;
  };
};

export const indexParsedDocument = async (filename: string) => {
  if (isTauriRuntime()) {
    await ensureDesktopBackendRunning();
    return invokeTauriCommand<{
      ok: boolean;
      filename: string;
      cards_indexed: number;
    }>('index_document', { filename });
  }

  const apiUrl = getApiUrl();
  const response = await axios.post(`${apiUrl}/index-document`, { filename });
  return response.data as {
    ok: boolean;
    filename: string;
    cards_indexed: number;
  };
};

export const getParserSettings = async () => {
  if (isTauriRuntime()) {
    await ensureDesktopBackendRunning();
    return invokeTauriCommand<{ settings: ParserSettings }>('get_parser_settings');
  }

  const apiUrl = getApiUrl();
  const response = await axios.get(`${apiUrl}/parser-settings`);
  return response.data as { settings: ParserSettings };
};

export const getParserEvents = async (limit = 120) => {
  if (isTauriRuntime()) {
    await ensureDesktopBackendRunning();
    return invokeTauriCommand<{ events: ParserEvent[] }>('get_parser_events', { limit });
  }

  const apiUrl = getApiUrl();
  const response = await axios.get(`${apiUrl}/parser-events?limit=${limit}`);
  return response.data as { events: ParserEvent[] };
};

export const updateParserSettings = async (settings: ParserSettings) => {
  if (isTauriRuntime()) {
    await ensureDesktopBackendRunning();
    return invokeTauriCommand<{ ok: boolean; settings: ParserSettings }>('update_parser_settings', { settings });
  }

  const apiUrl = getApiUrl();
  const response = await axios.post(`${apiUrl}/parser-settings`, settings);
  return response.data as { ok: boolean; settings: ParserSettings };
};

export const createUser = async (accessToken: string, refreshToken: string) => {
  const apiUrl = getApiUrl();
  await axios.post(`${apiUrl}/create-user`, { refresh_token: refreshToken }, { headers: { Authorization: `Bearer ${accessToken}` } });
};
