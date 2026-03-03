from dotenv import load_dotenv
import json
import os
import re
import heapq
import time
from datetime import datetime, timezone
from fcntl import flock, LOCK_EX, LOCK_UN

load_dotenv()

LOCAL_DOCS_FOLDER = os.environ.get("LOCAL_DOCS_FOLDER", "./local_docs")
LOCAL_INDEX_PATH = os.environ.get("LOCAL_INDEX_PATH", os.path.join(LOCAL_DOCS_FOLDER, "cards_index.json"))


def _to_unix_timestamp(date_value):
  if date_value in (None, ""):
    return None

  if isinstance(date_value, (int, float)):
    return int(date_value)

  value = str(date_value).strip()
  if value == "":
    return None

  try:
    return int(float(value))
  except ValueError:
    pass

  for date_format in ("%Y-%m-%d", "%Y/%m/%d"):
    try:
      dt = datetime.strptime(value, date_format).replace(tzinfo=timezone.utc)
      return int(dt.timestamp())
    except ValueError:
      continue

  return None


def _strip_identifier_token_from_tag(tag):
  return re.sub(r"\s*\[\[CID-[^\]]+\]\]\s*", " ", str(tag or ""), flags=re.IGNORECASE).strip()


def _normalize_exact_match_text(text):
  lowered = _strip_identifier_token_from_tag(text).lower()
  lowered = re.sub(r"[^a-z0-9\s]", " ", lowered)
  lowered = re.sub(r"\s+", " ", lowered)
  return lowered.strip()


def _normalize_query_fragment(value):
  normalized = _normalize_exact_match_text(value)
  return normalized


def _includes_all_tokens(text, terms, phrases):
  normalized = str(text or "").lower()
  return all(phrase in normalized for phrase in phrases) and all(term in normalized for term in terms)


def _is_tag_priority_match(card, terms, phrases):
  if len(terms) == 0 and len(phrases) == 0:
    return False

  tag_text = card.get("tag", "") or card.get("tag_base", "")
  candidate_text = " ".join([
    str(tag_text or ""),
    str(card.get("card_identifier", "") or ""),
    str(card.get("card_identifier_token", "") or ""),
    str(card.get("card_number", "") or ""),
  ])

  normalized_candidate = _normalize_exact_match_text(candidate_text)
  normalized_terms = [_normalize_query_fragment(term) for term in terms if _normalize_query_fragment(term)]
  normalized_phrases = [_normalize_query_fragment(phrase) for phrase in phrases if _normalize_query_fragment(phrase)]

  return all(phrase in normalized_candidate for phrase in normalized_phrases) and all(term in normalized_candidate for term in normalized_terms)


def _is_paragraph_match(card, terms, phrases):
  if len(terms) == 0 and len(phrases) == 0:
    return True

  body = card.get("body", [])
  if isinstance(body, list):
    body_text = " ".join([str(item) for item in body])
  else:
    body_text = str(body or "")

  paragraph_text = f"{str(card.get('highlighted_text', ''))} {body_text}"
  return _includes_all_tokens(paragraph_text, terms, phrases)


class Search:
  def __init__(self, index_path=LOCAL_INDEX_PATH):
    self.index_path = index_path
    os.makedirs(os.path.dirname(self.index_path), exist_ok=True)
    if not os.path.exists(self.index_path):
      self._reset_index_file()
    self._seen_ids = set()
    self._seen_filenames = set()
    self._index_loaded = False
    self._cards_cache = None
    self._cards_cache_signature = None
    self._id_lookup_cache = None
    self._id_lookup_signature = None
    self._query_state_cache = {}

  def _reset_index_file(self):
    with open(self.index_path, "w", encoding="utf-8") as handle:
      handle.write("")

  def _is_legacy_json_array(self):
    try:
      with open(self.index_path, "r", encoding="utf-8") as handle:
        while True:
          ch = handle.read(1)
          if ch == "":
            return False
          if not ch.isspace():
            return ch == "["
    except FileNotFoundError:
      return False

  def _append_card_dicts(self, card_dicts):
    if len(card_dicts) == 0:
      return

    with open(self.index_path, "a", encoding="utf-8") as handle:
      flock(handle.fileno(), LOCK_EX)
      try:
        for card in card_dicts:
          handle.write(json.dumps(card, ensure_ascii=False) + "\n")
      finally:
        flock(handle.fileno(), LOCK_UN)

  def _ensure_runtime_indexes(self):
    if self._index_loaded:
      return

    for card in self._read_cards():
      card_id = card.get("id")
      if card_id is not None:
        self._seen_ids.add(str(card_id))

      filename = str(card.get("filename", "")).strip().lower()
      if filename:
        self._seen_filenames.add(filename)

    self._index_loaded = True

  def _migrate_legacy_array_to_jsonl(self):
    if not self._is_legacy_json_array():
      return

    legacy_cards = self._read_cards()
    self._reset_index_file()
    self._append_card_dicts(legacy_cards)

  def _read_cards(self):
    try:
      with open(self.index_path, "r", encoding="utf-8") as handle:
        content = handle.read()
        if content.strip() == "":
          return []

      stripped = content.lstrip()
      if stripped.startswith("["):
        data = json.loads(content)
        if isinstance(data, list):
          return data
        return []

      cards = []
      for line in content.splitlines():
        line = line.strip()
        if not line:
          continue
        try:
          parsed = json.loads(line)
          if isinstance(parsed, dict):
            cards.append(parsed)
        except json.JSONDecodeError:
          continue
      return cards
    except (json.JSONDecodeError, FileNotFoundError):
      pass
    return []

  def _compute_index_signature(self):
    try:
      stat = os.stat(self.index_path)
      return (int(stat.st_mtime_ns), int(stat.st_size))
    except FileNotFoundError:
      return (0, 0)

  def _get_cards_snapshot(self):
    signature = self._compute_index_signature()
    if self._cards_cache is not None and self._cards_cache_signature == signature:
      return self._cards_cache

    cards = self._read_cards()
    self._cards_cache = cards
    self._cards_cache_signature = signature
    return cards

  def _invalidate_cards_cache(self):
    self._cards_cache = None
    self._cards_cache_signature = None
    self._id_lookup_cache = None
    self._id_lookup_signature = None
    self._query_state_cache = {}

  def _get_id_lookup(self):
    cards = self._get_cards_snapshot()
    signature = self._cards_cache_signature
    if self._id_lookup_cache is not None and self._id_lookup_signature == signature:
      return self._id_lookup_cache

    lookup = {}
    for card in cards:
      card_id = card.get("id")
      if card_id is None:
        continue
      lookup[str(card_id)] = card

    self._id_lookup_cache = lookup
    self._id_lookup_signature = signature
    return lookup

  def _query_state_key(self, q, start_date, end_date, exclude_sides, exclude_division, exclude_years, exclude_schools, sort_by, cite_match, match_mode):
    return "\u241f".join([
      str(q or ""),
      str(start_date or ""),
      str(end_date or ""),
      str(exclude_sides or ""),
      str(exclude_division or ""),
      str(exclude_years or ""),
      str(exclude_schools or ""),
      str(sort_by or ""),
      str(cite_match or ""),
      str(match_mode or ""),
    ])

  def _get_query_state(self, key):
    state = self._query_state_cache.get(key)
    if state is not None:
      state["last_used"] = time.time()
      return state

    state = {
      "matched_cards": [],
      "scan_index": 0,
      "exhausted": False,
      "last_used": time.time(),
    }
    self._query_state_cache[key] = state

    if len(self._query_state_cache) > 16:
      oldest_key = min(self._query_state_cache.keys(), key=lambda cache_key: self._query_state_cache[cache_key]["last_used"])
      self._query_state_cache.pop(oldest_key, None)

    return state

  def _get_search_blob(self, card):
    cached = card.get("_search_blob")
    if isinstance(cached, str):
      return cached

    blob = " ".join([
      str(card.get("tag", "")),
      str(card.get("card_identifier", "")),
      str(card.get("card_identifier_token", "")),
      str(card.get("card_number", "")),
      str(card.get("highlighted_text", "")),
      str(card.get("cite", "")),
      " ".join(card.get("body", []) if isinstance(card.get("body"), list) else [str(card.get("body", ""))]),
    ]).lower()
    card["_search_blob"] = blob
    return blob

  def get_all_cards(self):
    return self._get_cards_snapshot()

  def clear_index(self):
    self._reset_index_file()
    self._seen_ids.clear()
    self._seen_filenames.clear()
    self._index_loaded = True
    self._invalidate_cards_cache()

  def get_document_summaries(self):
    documents = {}
    for card in self._get_cards_snapshot():
      filename = str(card.get("filename", "")).strip()
      if not filename:
        continue

      key = filename.lower()
      if key not in documents:
        documents[key] = {
          "filename": filename,
          "cards_indexed": 0,
        }
      documents[key]["cards_indexed"] += 1

    return sorted(documents.values(), key=lambda item: item["filename"].lower())

  def delete_document_from_index(self, filename):
    target = str(filename).strip().lower()
    if target == "":
      return 0

    cards = self._get_cards_snapshot()
    kept_cards = []
    removed = 0

    for card in cards:
      card_filename = str(card.get("filename", "")).strip().lower()
      if card_filename == target:
        removed += 1
      else:
        kept_cards.append(card)

    self._reset_index_file()
    self._append_card_dicts(kept_cards)

    self._seen_ids.clear()
    self._seen_filenames.clear()
    for card in kept_cards:
      card_id = card.get("id")
      if card_id is not None:
        self._seen_ids.add(str(card_id))

      card_filename = str(card.get("filename", "")).strip().lower()
      if card_filename:
        self._seen_filenames.add(card_filename)
    self._index_loaded = True
    self._invalidate_cards_cache()

    return removed

  def check_filename_in_search(self, filename):
    if not filename:
      return False

    target = str(filename).strip().lower()
    if target == "":
      return False

    self._ensure_runtime_indexes()
    return target in self._seen_filenames

  def get_by_id(self, card_id):
    card_id = str(card_id)
    lookup = self._get_id_lookup()
    return lookup.get(card_id)

  def upload_cards(self, cards, force_upload=False):
    card_objects = [card.get_index() for card in cards]
    self.upload_card_indexes(card_objects, force_upload=force_upload)

  def upload_card_indexes(self, card_objects, force_upload=False):
    if len(card_objects) == 0:
      return

    self._migrate_legacy_array_to_jsonl()
    self._ensure_runtime_indexes()

    filename = card_objects[0].get("filename")
    normalized_filename = str(filename).strip().lower() if filename is not None else ""
    if normalized_filename and not force_upload and normalized_filename in self._seen_filenames:
      print(f"{filename} already in search, skipping")
      return

    to_append = []
    for card in card_objects:
      card_id = card.get("id")
      if card_id is None:
        continue
      card_id_str = str(card_id)
      if card_id_str in self._seen_ids:
        continue
      self._seen_ids.add(card_id_str)
      to_append.append(card)

    self._append_card_dicts(to_append)

    if len(to_append) > 0 and self._cards_cache is not None:
      self._cards_cache.extend(to_append)
      self._cards_cache_signature = self._compute_index_signature()
      if self._id_lookup_cache is not None:
        for card in to_append:
          card_id = card.get("id")
          if card_id is not None:
            self._id_lookup_cache[str(card_id)] = card

    self._query_state_cache = {}

    if normalized_filename:
      self._seen_filenames.add(normalized_filename)

    if filename is not None:
      print(f"Indexed locally: {filename}")
    else:
      print("Indexed locally")

  def upload_to_dynamo(self, cards):
    self.upload_cards(cards)

  def query(self, q, from_value=0, start_date="", end_date="", exclude_sides="", exclude_division="", exclude_years="", exclude_schools="", sort_by="", cite_match="", limit=30, match_mode=""):
    cards = self._get_cards_snapshot()

    quoted_phrases = []
    remaining_text = q or ""
    while '"' in remaining_text:
      first = remaining_text.find('"')
      second = remaining_text.find('"', first + 1)
      if second == -1:
        break
      phrase = remaining_text[first + 1:second].strip().lower()
      if phrase:
        quoted_phrases.append(phrase)
      remaining_text = f"{remaining_text[:first]} {remaining_text[second + 1:]}"

    terms = [term.strip().lower() for term in remaining_text.split() if term.strip()]

    excluded_sides = set(s.strip().lower() for s in str(exclude_sides).split(",") if s.strip())
    excluded_divisions = set(d.split("-")[0].strip().lower() for d in str(exclude_division).split(",") if d.strip())
    excluded_years_set = set(y.strip().lower() for y in str(exclude_years).split(",") if y.strip())
    excluded_schools_set = set(s.strip().lower() for s in str(exclude_schools).split(",") if s.strip())
    normalized_match_mode = str(match_mode or "").strip().lower()

    start_ts = _to_unix_timestamp(start_date)
    end_ts = _to_unix_timestamp(end_date)

    try:
      offset = max(0, int(from_value))
    except (TypeError, ValueError):
      offset = 0
    safe_limit = max(1, int(limit)) if str(limit).strip() != "" else 30

    normalized_sort_by = str(sort_by or "").strip().lower()
    requires_date_sort = normalized_sort_by == "date"

    page = []
    total_count = 0
    window_size = offset + safe_limit if requires_date_sort else 0
    top_heap = []

    def card_matches(card):
      filename = str(card.get("filename", "")).lower()
      division = str(card.get("division", "")).lower()
      year = str(card.get("year", "")).lower()
      school = str(card.get("school", "")).lower()
      cite = str(card.get("cite", ""))

      if excluded_sides and any(side in filename for side in excluded_sides):
        return False
      if excluded_divisions and division in excluded_divisions:
        return False
      if excluded_years_set and year in excluded_years_set:
        return False
      if excluded_schools_set and school in excluded_schools_set:
        return False

      if start_ts is not None and end_ts is not None:
        card_date = card.get("cite_date")
        card_ts = _to_unix_timestamp(card_date)
        if card_ts is None or card_ts < start_ts or card_ts > end_ts:
          return False

      if cite_match and str(cite_match).lower() not in cite.lower():
        return False

      if normalized_match_mode == "tag":
        return _is_tag_priority_match(card, terms, quoted_phrases)

      if normalized_match_mode == "paragraph":
        return _is_paragraph_match(card, terms, quoted_phrases)

      searchable_text = self._get_search_blob(card)
      if any(phrase not in searchable_text for phrase in quoted_phrases):
        return False
      if any(term not in searchable_text for term in terms):
        return False
      return True

    if not requires_date_sort:
      target = offset + safe_limit
      state_key = self._query_state_key(
        q,
        start_date,
        end_date,
        exclude_sides,
        exclude_division,
        exclude_years,
        exclude_schools,
        sort_by,
        cite_match,
        normalized_match_mode,
      )
      state = self._get_query_state(state_key)

      while len(state["matched_cards"]) < target and not state["exhausted"]:
        while state["scan_index"] < len(cards) and len(state["matched_cards"]) < target:
          card = cards[state["scan_index"]]
          state["scan_index"] += 1
          if card_matches(card):
            state["matched_cards"].append(card)

        if state["scan_index"] >= len(cards):
          state["exhausted"] = True

      page = state["matched_cards"][offset:offset + safe_limit]
      cursor = offset + len(page)
      has_more = len(state["matched_cards"]) > cursor or not state["exhausted"]
      total_count = len(state["matched_cards"]) if state["exhausted"] else len(state["matched_cards"])
      return page, cursor, total_count, has_more, (not state["exhausted"])

    for card in cards:
      if not card_matches(card):
        continue

      total_count += 1
      if not requires_date_sort:
        if total_count <= offset:
          continue
        if len(page) < safe_limit:
          page.append(card)
      else:
        if window_size <= 0:
          continue

        card_timestamp = _to_unix_timestamp(card.get("cite_date")) or 0
        heap_item = (card_timestamp, total_count, card)
        if len(top_heap) < window_size:
          heapq.heappush(top_heap, heap_item)
        else:
          smallest_timestamp, smallest_order, _ = top_heap[0]
          if card_timestamp > smallest_timestamp or (
            card_timestamp == smallest_timestamp and total_count > smallest_order
          ):
            heapq.heapreplace(top_heap, heap_item)

    if requires_date_sort and len(top_heap) > 0:
      sorted_top = sorted(top_heap, key=lambda item: (item[0], item[1]), reverse=True)
      page = [item[2] for item in sorted_top[offset:offset + safe_limit]]

    cursor = offset + len(page)
    has_more = cursor < total_count
    return page, cursor, total_count, has_more, False

  def get_colleges(self):
    schools = sorted({str(card.get("school", "")).strip() for card in self._get_cards_snapshot() if str(card.get("school", "")).strip()})
    return schools
