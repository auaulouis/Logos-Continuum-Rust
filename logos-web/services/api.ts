import axios from 'axios';

const oldUrl = 'https://logos-web.onrender.com';
const newUrl = 'https://logos-debate.duckdns.org';
const desktopApiUrl = 'http://127.0.0.1:5501';

const getApiUrl = () => {
  const isElectronRenderer = typeof navigator !== 'undefined' && navigator.userAgent.includes('Electron');
  if (isElectronRenderer) {
    return desktopApiUrl;
  }

  if (process.env.NEXT_PUBLIC_API_URL) {
    return process.env.NEXT_PUBLIC_API_URL;
  }

  return process.env.NODE_ENV === 'development' ? 'http://localhost:5001' : newUrl;
};

export const search = async (query: string, cursor = 0, additionalParams = {}, limit = 30) => {
  const apiUrl = getApiUrl();
  let url = `${apiUrl}/query?search=${query}&cursor=${cursor}&limit=${limit}`;
  Object.entries(additionalParams).forEach(([key, value]) => {
    url += `&${key}=${value}`;
  });

  const response = await axios.get(url);
  return {
    results: response.data.results,
    cursor: response.data.cursor,
    totalCount: response.data.total_count,
    hasMore: Boolean(response.data.has_more),
    countIsPartial: Boolean(response.data.count_is_partial),
  };
};

export const getCard = async (id: string) => {
  const apiUrl = getApiUrl();
  const response = await axios.get(`${apiUrl}/card?id=${id}`);
  return response.data;
};

export const getSchools = async () => {
  const apiUrl = getApiUrl();
  const response = await axios.get(`${apiUrl}/schools`);
  return response.data;
};

export const uploadDocx = async (file: File, options?: { parseImmediately?: boolean }) => {
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
  const apiUrl = getApiUrl();
  const response = await axios.post(`${apiUrl}/parse-uploaded-docs`);
  return response.data as {
    ok: boolean;
    queued: number;
    skipped_already_indexed: number;
  };
};

export const clearIndex = async () => {
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
  const apiUrl = getApiUrl();
  const response = await axios.get(`${apiUrl}/documents`);
  return response.data as { documents: ParsedDocument[] };
};

export const deleteParsedDocument = async (filename: string, target: 'index' | 'folder') => {
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
  const apiUrl = getApiUrl();
  const response = await axios.post(`${apiUrl}/index-document`, { filename });
  return response.data as {
    ok: boolean;
    filename: string;
    cards_indexed: number;
  };
};

export const getParserSettings = async () => {
  const apiUrl = getApiUrl();
  const response = await axios.get(`${apiUrl}/parser-settings`);
  return response.data as { settings: ParserSettings };
};

export const getParserEvents = async (limit = 120) => {
  const apiUrl = getApiUrl();
  const response = await axios.get(`${apiUrl}/parser-events?limit=${limit}`);
  return response.data as { events: ParserEvent[] };
};

export const updateParserSettings = async (settings: ParserSettings) => {
  const apiUrl = getApiUrl();
  const response = await axios.post(`${apiUrl}/parser-settings`, settings);
  return response.data as { ok: boolean; settings: ParserSettings };
};

export const createUser = async (accessToken: string, refreshToken: string) => {
  const apiUrl = getApiUrl();
  await axios.post(`${apiUrl}/create-user`, { refresh_token: refreshToken }, { headers: { Authorization: `Bearer ${accessToken}` } });
};
