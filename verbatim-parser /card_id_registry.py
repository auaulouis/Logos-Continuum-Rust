import json
import os
from fcntl import flock, LOCK_EX, LOCK_UN


LOCAL_DOCS_FOLDER = os.environ.get("LOCAL_DOCS_FOLDER", "./local_docs")
CARD_ID_REGISTRY_PATH = os.environ.get("CARD_ID_REGISTRY_PATH", os.path.join(LOCAL_DOCS_FOLDER, "card_id_registry.json"))


class CardIdRegistry:
  def __init__(self, registry_path=None):
    self.registry_path = registry_path or os.environ.get("CARD_ID_REGISTRY_PATH", CARD_ID_REGISTRY_PATH)
    os.makedirs(os.path.dirname(self.registry_path), exist_ok=True)
    if not os.path.exists(self.registry_path):
      self._write_registry({"next_number": 1, "used_numbers": [], "assigned_by_key": {}})

  def _read_registry_unlocked(self, handle):
    handle.seek(0)
    raw = handle.read().strip()
    if raw == "":
      return {"next_number": 1, "used_numbers": [], "assigned_by_key": {}}

    try:
      parsed = json.loads(raw)
      if not isinstance(parsed, dict):
        return {"next_number": 1, "used_numbers": [], "assigned_by_key": {}}
    except json.JSONDecodeError:
      return {"next_number": 1, "used_numbers": [], "assigned_by_key": {}}

    next_number = parsed.get("next_number", 1)
    used_numbers = parsed.get("used_numbers", [])
    assigned_by_key = parsed.get("assigned_by_key", {})

    if not isinstance(next_number, int) or next_number < 1:
      next_number = 1
    if not isinstance(used_numbers, list):
      used_numbers = []
    if not isinstance(assigned_by_key, dict):
      assigned_by_key = {}

    return {
      "next_number": next_number,
      "used_numbers": [int(v) for v in used_numbers if isinstance(v, int) and v > 0],
      "assigned_by_key": {str(k): int(v) for k, v in assigned_by_key.items() if isinstance(v, int) and v > 0},
    }

  def _write_registry_unlocked(self, handle, data):
    handle.seek(0)
    handle.truncate()
    json.dump(data, handle, ensure_ascii=False, indent=2)
    handle.flush()

  def _write_registry(self, data):
    with open(self.registry_path, "w", encoding="utf-8") as handle:
      json.dump(data, handle, ensure_ascii=False, indent=2)

  def get_or_assign_numbers(self, card_keys):
    keys = [str(key).strip() for key in card_keys if str(key).strip() != ""]
    if len(keys) == 0:
      return {}

    numbers_by_key = {}
    with open(self.registry_path, "a+", encoding="utf-8") as handle:
      flock(handle.fileno(), LOCK_EX)
      try:
        registry = self._read_registry_unlocked(handle)
        assigned_by_key = registry["assigned_by_key"]
        used_numbers = registry["used_numbers"]
        next_number = registry["next_number"]

        seen = set()
        for key in keys:
          if key in seen:
            numbers_by_key[key] = assigned_by_key.get(key)
            continue
          seen.add(key)

          existing = assigned_by_key.get(key)
          if existing is not None:
            numbers_by_key[key] = existing
            continue

          assigned_by_key[key] = next_number
          used_numbers.append(next_number)
          numbers_by_key[key] = next_number
          next_number += 1

        registry["next_number"] = next_number
        registry["used_numbers"] = used_numbers
        registry["assigned_by_key"] = assigned_by_key
        self._write_registry_unlocked(handle, registry)
      finally:
        flock(handle.fileno(), LOCK_UN)

    return numbers_by_key
