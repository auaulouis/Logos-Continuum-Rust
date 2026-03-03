from flask import Flask, request
from flask_cors import CORS
from dotenv import load_dotenv
from search import Search
from parser import Parser, resolve_card_workers
import os
import glob
import asyncio
import time
import json
from concurrent.futures import ThreadPoolExecutor
from threading import Lock
from werkzeug.utils import secure_filename

load_dotenv()

app = Flask(__name__)
CORS(app)

LOCAL_DOCS_FOLDER = os.environ.get("LOCAL_DOCS_FOLDER", "./local_docs")
PARSER_SETTINGS_PATH = os.environ.get("PARSER_SETTINGS_PATH", os.path.join(LOCAL_DOCS_FOLDER, "parser_settings.json"))
PARSER_EVENTS_PATH = os.environ.get("PARSER_EVENTS_PATH", os.path.join(LOCAL_DOCS_FOLDER, "parser_events.jsonl"))


DEFAULT_PARSER_SETTINGS = {
  "use_parallel_processing": True,
  "parser_card_workers": resolve_card_workers(),
  "local_parser_file_workers": max(1, min(os.cpu_count() or 1, 8)),
  "flush_enabled": True,
  "flush_every_docs": 250,
}

_UPLOAD_PARSE_WORKERS = max(1, int(os.environ.get("UPLOAD_PARSE_WORKERS", "2")))
_UPLOAD_PARSE_EXECUTOR = ThreadPoolExecutor(max_workers=_UPLOAD_PARSE_WORKERS)
_UPLOAD_PARSE_LOCK = Lock()
_UPLOAD_PARSE_ACTIVE = set()


def _append_parser_event(level, message, payload=None):
  event = {
    "id": f"{int(time.time() * 1000)}-{os.getpid()}-{(time.time_ns() % 1000000)}",
    "at": int(time.time() * 1000),
    "level": str(level),
    "message": str(message),
  }
  if isinstance(payload, dict):
    event.update(payload)

  try:
    os.makedirs(os.path.dirname(PARSER_EVENTS_PATH), exist_ok=True)
    with open(PARSER_EVENTS_PATH, "a", encoding="utf-8") as handle:
      handle.write(json.dumps(event, ensure_ascii=False) + "\n")
  except OSError:
    pass

  print(f"[parser:{event['level']}] {event['message']}", flush=True)
  return event


def _tail_parser_events(limit=120):
  if not os.path.isfile(PARSER_EVENTS_PATH):
    return []

  try:
    with open(PARSER_EVENTS_PATH, "r", encoding="utf-8") as handle:
      lines = handle.readlines()
  except OSError:
    return []

  events = []
  for line in lines[-max(1, int(limit)):]:
    raw = line.strip()
    if raw == "":
      continue
    try:
      parsed = json.loads(raw)
      if isinstance(parsed, dict):
        events.append(parsed)
    except json.JSONDecodeError:
      continue

  return events


def _coerce_bool(value, fallback):
  if isinstance(value, bool):
    return value
  if isinstance(value, str):
    normalized = value.strip().lower()
    if normalized in ("true", "1", "yes", "on"):
      return True
    if normalized in ("false", "0", "no", "off"):
      return False
  if isinstance(value, (int, float)):
    return value != 0
  return fallback


def _clamp_worker_count(value, fallback):
  max_workers = max(1, os.cpu_count() or 1)
  try:
    return max(1, min(int(value), max_workers))
  except (TypeError, ValueError):
    return max(1, min(int(fallback), max_workers))


def _normalize_parser_settings(data):
  settings = dict(DEFAULT_PARSER_SETTINGS)
  if not isinstance(data, dict):
    return settings

  if "use_parallel_processing" in data:
    settings["use_parallel_processing"] = _coerce_bool(
      data.get("use_parallel_processing"),
      settings["use_parallel_processing"],
    )

  if "flush_enabled" in data:
    settings["flush_enabled"] = _coerce_bool(data.get("flush_enabled"), settings["flush_enabled"])

  settings["parser_card_workers"] = _clamp_worker_count(
    data.get("parser_card_workers", settings["parser_card_workers"]),
    settings["parser_card_workers"],
  )

  settings["local_parser_file_workers"] = _clamp_worker_count(
    data.get("local_parser_file_workers", settings["local_parser_file_workers"]),
    settings["local_parser_file_workers"],
  )

  try:
    settings["flush_every_docs"] = max(1, int(data.get("flush_every_docs", settings["flush_every_docs"])))
  except (TypeError, ValueError):
    pass

  return settings


def _load_parser_settings():
  try:
    with open(PARSER_SETTINGS_PATH, "r", encoding="utf-8") as handle:
      content = handle.read().strip()
      if content == "":
        return dict(DEFAULT_PARSER_SETTINGS)
      parsed = json.loads(content)
      return _normalize_parser_settings(parsed)
  except (FileNotFoundError, json.JSONDecodeError, OSError):
    return dict(DEFAULT_PARSER_SETTINGS)


def _save_parser_settings(settings):
  normalized = _normalize_parser_settings(settings)
  os.makedirs(os.path.dirname(PARSER_SETTINGS_PATH), exist_ok=True)
  with open(PARSER_SETTINGS_PATH, "w", encoding="utf-8") as handle:
    json.dump(normalized, handle, ensure_ascii=False, indent=2)
  return normalized


def _resolve_api_card_workers():
  env_workers = os.environ.get("PARSER_CARD_WORKERS")
  if env_workers is not None:
    return resolve_card_workers()

  settings = _load_parser_settings()
  if not settings.get("use_parallel_processing", True):
    return 1
  return max(1, int(settings.get("parser_card_workers", resolve_card_workers())))


def _uploaded_docs_root():
  return os.path.join(LOCAL_DOCS_FOLDER, "uploaded_docs")


def _list_uploaded_docx_files():
  upload_dir = _uploaded_docs_root()
  if not os.path.isdir(upload_dir):
    return []

  files = glob.glob(os.path.join(upload_dir, "**/*.docx"), recursive=True)
  files = [path for path in files if not os.path.basename(path).startswith("~$")]

  items = []
  for path in files:
    relative_path = os.path.relpath(path, start=upload_dir)
    items.append({
      "filename": os.path.basename(path),
      "relative_path": relative_path,
      "absolute_path": path,
    })
  return items


def _delete_uploaded_docx_file(filename):
  target = str(filename).strip().lower()
  if target == "":
    return None

  for item in _list_uploaded_docx_files():
    if item["filename"].strip().lower() == target:
      os.remove(item["absolute_path"])
      return item["relative_path"]
  return None


def _find_uploaded_docx_file(filename):
  target = str(filename).strip().lower()
  if target == "":
    return None

  for item in _list_uploaded_docx_files():
    if item["filename"].strip().lower() == target:
      return item
  return None


def _parse_bool(value, fallback=True):
  if value is None:
    return fallback
  normalized = str(value).strip().lower()
  if normalized in ("1", "true", "yes", "on"):
    return True
  if normalized in ("0", "false", "no", "off"):
    return False
  return fallback


def _run_background_parse(saved_path, stored_filename):
  parse_started = time.perf_counter()
  try:
    parser = Parser(saved_path, {
      "filename": stored_filename,
      "division": "local",
      "year": "local",
      "school": "Local",
      "team": "Local",
      "download_url": "local"
    },
      max_workers=_resolve_api_card_workers(),
      profile=os.environ.get("PARSER_PROFILE", "0") == "1"
    )
    cards = parser.parse()
    parse_ms = (time.perf_counter() - parse_started) * 1000
    cards_per_second = (len(cards) * 1000 / parse_ms) if parse_ms > 0 else 0

    search = _get_api_instance().search
    search.upload_cards(cards, force_upload=True)
    _append_parser_event(
      "info",
      f"Parsed {stored_filename}: {len(cards)} cards in {parse_ms:.2f}ms ({cards_per_second:.2f} cards/s)",
      {
        "source": "api-upload",
        "filename": stored_filename,
        "cards_indexed": len(cards),
        "parse_ms": round(parse_ms, 2),
        "cards_per_second": round(cards_per_second, 2),
      },
    )
  except Exception as error:
    _append_parser_event("error", f"Failed parsing {stored_filename}: {error}", {
      "source": "api-upload",
      "filename": stored_filename,
    })
  finally:
    with _UPLOAD_PARSE_LOCK:
      _UPLOAD_PARSE_ACTIVE.discard(stored_filename)


def _enqueue_background_parse(saved_path, stored_filename):
  with _UPLOAD_PARSE_LOCK:
    if stored_filename in _UPLOAD_PARSE_ACTIVE:
      return False
    _UPLOAD_PARSE_ACTIVE.add(stored_filename)

  _UPLOAD_PARSE_EXECUTOR.submit(_run_background_parse, saved_path, stored_filename)
  return True


def _queue_startup_index_files(files):
  queued = 0
  for path in files:
    filename = os.path.basename(path)
    if _enqueue_background_parse(path, filename):
      queued += 1
  return queued


def _queue_parse_for_uploaded_docs(search_client):
  queued = 0
  skipped_already_indexed = 0

  for item in _list_uploaded_docx_files():
    filename = item.get("filename", "")
    absolute_path = item.get("absolute_path", "")

    if not filename or not absolute_path:
      continue

    if search_client.check_filename_in_search(filename):
      skipped_already_indexed += 1
      continue

    if _enqueue_background_parse(absolute_path, filename):
      queued += 1

  return queued, skipped_already_indexed


def _index_local_docs_if_empty(search_client):
  if len(search_client.get_all_cards()) > 0:
    return

  files = glob.glob(os.path.join(LOCAL_DOCS_FOLDER, "**/*.docx"), recursive=True)
  files = [path for path in files if not os.path.basename(path).startswith("~$")]

  if len(files) == 0:
    print(f"No local .docx files found in {LOCAL_DOCS_FOLDER}; starting API with empty index")
    return

  print(f"Local index is empty. Queueing {len(files)} local .docx files for background parse...")
  _append_parser_event("info", f"Startup parse begins for {len(files)} local file(s)", {
    "source": "api-startup",
    "files": len(files),
  })
  queued = _queue_startup_index_files(files)
  _append_parser_event("info", f"Startup parse queued {queued}/{len(files)} file(s)", {
    "source": "api-startup",
    "files": len(files),
    "queued": queued,
  })


class Api:
  def __init__(self):
    self.search = Search()
    _index_local_docs_if_empty(self.search)

  async def query(self, q, from_value=0, start_date="", end_date="", exclude_sides="", exclude_division="", exclude_years="", exclude_schools="", sort_by="", cite_match="", limit=30, match_mode=""):
    return self.search.query(
      q,
      from_value=from_value,
      start_date=start_date,
      end_date=end_date,
      exclude_sides=exclude_sides,
      exclude_division=exclude_division,
      exclude_years=exclude_years,
      exclude_schools=exclude_schools,
      sort_by=sort_by,
      cite_match=cite_match,
      limit=limit,
      match_mode=match_mode
    )

  def get_colleges(self):
    return self.search.get_colleges()

  async def get_by_id(self, card_id, preview=False):
    card_data = self.search.get_by_id(card_id)
    if card_data is None:
      return None

    if "highlights" not in card_data:
      card_data["highlights"] = []
    if "underlines" not in card_data:
      card_data["underlines"] = []
    if "emphasis" not in card_data:
      card_data["emphasis"] = []

    return card_data


_API_INSTANCE = None


def _get_api_instance():
  global _API_INSTANCE
  if _API_INSTANCE is None:
    _API_INSTANCE = Api()
  return _API_INSTANCE


@app.route("/query", methods=['GET'])
def query():
  search = request.args.get('search', '')
  cursor = int(request.args.get('cursor', 0))
  start_date = request.args.get('start_date', '')
  end_date = request.args.get('end_date', '')
  exclude_sides = request.args.get('exclude_sides', '')
  exclude_division = request.args.get('exclude_division', '')
  exclude_schools = request.args.get('exclude_schools', '')
  exclude_years = request.args.get('exclude_years', '')
  sort_by = request.args.get('sort_by', '')
  cite_match = request.args.get('cite_match', '')
  match_mode = request.args.get('match_mode', '')
  limit = max(1, min(int(request.args.get('limit', 30)), 30))

  api = _get_api_instance()
  results, next_cursor, total_count, has_more, count_is_partial = asyncio.run(api.query(
    search,
    cursor,
    start_date=start_date,
    end_date=end_date,
    exclude_sides=exclude_sides,
    exclude_division=exclude_division,
    exclude_schools=exclude_schools,
    exclude_years=exclude_years,
    sort_by=sort_by,
    cite_match=cite_match,
    limit=limit,
    match_mode=match_mode
  ))
  return {
    "count": len(results),
    "results": results,
    "cursor": next_cursor,
    "total_count": total_count,
    "has_more": has_more,
    "count_is_partial": count_is_partial,
  }


@app.route("/card", methods=['GET'])
def get_card():
  card_id = request.args.get('id')
  api = _get_api_instance()
  result = asyncio.run(api.get_by_id(card_id, False))
  return result


@app.route("/schools", methods=['GET'])
def get_schools_list():
  api = _get_api_instance()
  schools = api.get_colleges()
  return {"colleges": schools}


@app.route("/clear-index", methods=['POST'])
def clear_index():
  search = _get_api_instance().search
  search.clear_index()
  return {"ok": True}


@app.route("/parser-settings", methods=['GET'])
def get_parser_settings():
  settings = _load_parser_settings()
  return {"settings": settings}


@app.route("/parser-settings", methods=['POST'])
def update_parser_settings():
  payload = request.get_json(silent=True) or {}
  updated = _save_parser_settings(payload)
  return {"ok": True, "settings": updated}


@app.route("/parser-events", methods=['GET'])
def get_parser_events():
  try:
    limit = int(request.args.get('limit', 120))
  except (TypeError, ValueError):
    limit = 120

  limit = max(1, min(limit, 500))
  return {"events": _tail_parser_events(limit)}


@app.route("/upload-docx", methods=['POST'])
def upload_docx():
  uploaded_file = request.files.get('file')
  if uploaded_file is None or uploaded_file.filename is None or uploaded_file.filename.strip() == "":
    return {"error": "No file uploaded"}, 400

  original_filename = secure_filename(uploaded_file.filename)
  if not original_filename.lower().endswith('.docx'):
    return {"error": "Only .docx files are supported"}, 400

  upload_dir = os.path.join(LOCAL_DOCS_FOLDER, "uploaded_docs")
  os.makedirs(upload_dir, exist_ok=True)

  base_name, ext = os.path.splitext(original_filename)
  saved_path = os.path.join(upload_dir, original_filename)
  suffix = 1
  while os.path.exists(saved_path):
    saved_path = os.path.join(upload_dir, f"{base_name}-{suffix}{ext}")
    suffix += 1

  uploaded_file.save(saved_path)
  stored_filename = os.path.basename(saved_path)
  _append_parser_event("info", f"Upload received: {stored_filename}", {
    "source": "api-upload",
    "filename": stored_filename,
  })

  parse_immediately = _parse_bool(request.form.get("parse"), True)

  if not parse_immediately:
    _append_parser_event("info", f"Upload stored (deferred parse): {stored_filename}", {
      "source": "api-upload",
      "filename": stored_filename,
    })
    return {
      "ok": True,
      "queued": False,
      "deferred": True,
      "filename": stored_filename,
      "stored_path": saved_path,
      "cards_indexed": 0,
      "parse_ms": 0,
    }

  queued = _enqueue_background_parse(saved_path, stored_filename)
  if queued:
    _append_parser_event("info", f"Queued parsing: {stored_filename}", {
      "source": "api-upload",
      "filename": stored_filename,
    })
    return {
      "ok": True,
      "queued": True,
      "filename": stored_filename,
      "stored_path": saved_path,
      "cards_indexed": 0,
      "parse_ms": 0,
    }

  return {
    "ok": True,
    "queued": False,
    "deferred": False,
    "filename": stored_filename,
    "stored_path": saved_path,
    "cards_indexed": 0,
    "parse_ms": 0,
  }


@app.route("/parse-uploaded-docs", methods=['POST'])
def parse_uploaded_docs():
  search = _get_api_instance().search
  queued, skipped = _queue_parse_for_uploaded_docs(search)
  _append_parser_event("info", f"Batch parse queued {queued} uploaded doc(s), skipped {skipped} already indexed", {
    "source": "api-batch-parse",
    "queued": queued,
    "skipped": skipped,
  })
  return {
    "ok": True,
    "queued": queued,
    "skipped_already_indexed": skipped,
  }


@app.route("/documents", methods=['GET'])
def list_documents():
  search = _get_api_instance().search
  indexed_docs = search.get_document_summaries()
  indexed_by_name = {str(doc.get("filename", "")).strip().lower(): doc for doc in indexed_docs}

  uploaded_docs = _list_uploaded_docx_files()
  uploaded_by_name = {}
  for doc in uploaded_docs:
    key = doc["filename"].strip().lower()
    if key not in uploaded_by_name:
      uploaded_by_name[key] = doc

  keys = sorted(set(indexed_by_name.keys()) | set(uploaded_by_name.keys()))
  documents = []
  for key in keys:
    indexed_doc = indexed_by_name.get(key)
    uploaded_doc = uploaded_by_name.get(key)

    filename = ""
    if indexed_doc is not None:
      filename = indexed_doc.get("filename", "")
    elif uploaded_doc is not None:
      filename = uploaded_doc.get("filename", "")

    documents.append({
      "filename": filename,
      "cards_indexed": int(indexed_doc.get("cards_indexed", 0)) if indexed_doc is not None else 0,
      "in_index": indexed_doc is not None,
      "in_folder": uploaded_doc is not None,
      "folder_path": uploaded_doc.get("relative_path") if uploaded_doc is not None else None,
    })

  return {"documents": documents}


@app.route("/delete-document", methods=['POST'])
def delete_document():
  payload = request.get_json(silent=True) or {}
  filename = str(payload.get("filename", "")).strip()
  target = str(payload.get("target", "")).strip().lower()

  if filename == "":
    return {"error": "filename is required"}, 400

  if target not in ("index", "folder"):
    return {"error": "target must be either 'index' or 'folder'"}, 400

  removed_cards = 0
  removed_from_folder = False
  deleted_path = None

  if target == "index":
    search = _get_api_instance().search
    removed_cards = search.delete_document_from_index(filename)
  else:
    deleted_path = _delete_uploaded_docx_file(filename)
    removed_from_folder = deleted_path is not None

  if removed_cards == 0 and not removed_from_folder:
    return {
      "ok": False,
      "removed_cards": 0,
      "removed_from_folder": False,
      "deleted_path": None,
      "message": "Document not found for selected target",
    }, 404

  return {
    "ok": True,
    "removed_cards": removed_cards,
    "removed_from_folder": removed_from_folder,
    "deleted_path": deleted_path,
  }


@app.route("/index-document", methods=['POST'])
def index_document():
  payload = request.get_json(silent=True) or {}
  filename = str(payload.get("filename", "")).strip()
  if filename == "":
    return {"error": "filename is required"}, 400

  file_item = _find_uploaded_docx_file(filename)
  if file_item is None:
    return {"error": "Document file not found in uploaded_docs"}, 404

  absolute_path = file_item["absolute_path"]
  try:
    parse_started = time.perf_counter()
    parser = Parser(absolute_path, {
      "filename": file_item["filename"],
      "division": "local",
      "year": "local",
      "school": "Local",
      "team": "Local",
      "download_url": "local"
    },
      max_workers=_resolve_api_card_workers(),
      profile=os.environ.get("PARSER_PROFILE", "0") == "1"
    )
    cards = parser.parse()
    parse_ms = (time.perf_counter() - parse_started) * 1000
    cards_per_second = (len(cards) * 1000 / parse_ms) if parse_ms > 0 else 0
    search = _get_api_instance().search
    search.upload_cards(cards, force_upload=True)
    _append_parser_event(
      "info",
      f"Indexed {file_item['filename']}: {len(cards)} cards in {parse_ms:.2f}ms ({cards_per_second:.2f} cards/s)",
      {
        "source": "api-index-document",
        "filename": file_item["filename"],
        "cards_indexed": len(cards),
        "parse_ms": round(parse_ms, 2),
        "cards_per_second": round(cards_per_second, 2),
      },
    )
    return {
      "ok": True,
      "filename": file_item["filename"],
      "cards_indexed": len(cards),
    }
  except Exception as error:
    _append_parser_event("error", f"Failed indexing {file_item['filename']}: {error}", {
      "source": "api-index-document",
      "filename": file_item["filename"],
    })
    return {"error": f"Failed to index {file_item['filename']}: {error}"}, 500


if __name__ == '__main__':
  app.run(port=int(os.environ.get('PORT', '5001')), host='0.0.0.0', debug=True)
