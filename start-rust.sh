#!/bin/bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
BACKEND_DIR="$ROOT_DIR/rust-backend"
FRONTEND_DIR="$ROOT_DIR/logos-web"
LOCAL_DOCS_DIR="${LOCAL_DOCS_DIR:-$ROOT_DIR/local_docs_dev}"
MODE="${1:-dev}"
BACKEND_API_URL="${BACKEND_API_URL:-http://127.0.0.1:5002}"

backend_is_up() {
  curl -fsS --max-time 2 "$BACKEND_API_URL/health" >/dev/null 2>&1
}

run_backend() {
  echo "🦀 Starting Rust API on Port 5002..."
  mkdir -p "$LOCAL_DOCS_DIR/uploaded_docs"
  cd "$BACKEND_DIR"
  LOCAL_DOCS_FOLDER="$LOCAL_DOCS_DIR" \
  LOCAL_INDEX_PATH="$LOCAL_DOCS_DIR/cards_index.json" \
  CARD_ID_REGISTRY_PATH="$LOCAL_DOCS_DIR/card_id_registry.json" \
  PARSER_SETTINGS_PATH="$LOCAL_DOCS_DIR/parser_settings.json" \
  PARSER_EVENTS_PATH="$LOCAL_DOCS_DIR/parser_events.jsonl" \
  PORT=5002 cargo run --bin logos-backend &
  BACKEND_PID=$!
}

run_pipeline_bin() {
  local BIN_NAME="$1"
  local STARTED_BACKEND=0

  cleanup() {
    echo "🛑 Shutting down..."
    if [ "$STARTED_BACKEND" -eq 1 ]; then
      [ -n "${BACKEND_PID:-}" ] && kill "$BACKEND_PID" 2>/dev/null || true
    fi
  }

  trap cleanup SIGINT SIGTERM EXIT

  if backend_is_up; then
    echo "♻️  Reusing existing Rust API at $BACKEND_API_URL"
  else
    run_backend
    STARTED_BACKEND=1
    sleep 1
  fi

  echo "⚙️  Running Rust pipeline: $BIN_NAME"
  cd "$BACKEND_DIR"
  LOCAL_DOCS_FOLDER="$LOCAL_DOCS_DIR" \
  LOCAL_INDEX_PATH="$LOCAL_DOCS_DIR/cards_index.json" \
  PARSER_SETTINGS_PATH="$LOCAL_DOCS_DIR/parser_settings.json" \
  PARSER_EVENTS_PATH="$LOCAL_DOCS_DIR/parser_events.jsonl" \
  BACKEND_API_URL="$BACKEND_API_URL" \
  cargo run --bin "$BIN_NAME"
}

echo "🚀 Starting Logos with Rust backend..."

if [ ! -d "$BACKEND_DIR" ]; then
  echo "❌ Rust backend directory not found: $BACKEND_DIR"
  exit 1
fi

if [ ! -d "$FRONTEND_DIR" ]; then
  echo "❌ Frontend directory not found: $FRONTEND_DIR"
  exit 1
fi

if [ "$MODE" = "local-parser" ] || [ "$MODE" = "local_parser" ]; then
  run_pipeline_bin "local_parser"
  exit 0
fi

if [ "$MODE" = "scraper" ]; then
  run_pipeline_bin "scraper"
  exit 0
fi

if [ "$MODE" = "main-pipeline" ] || [ "$MODE" = "main_pipeline" ]; then
  run_pipeline_bin "main_pipeline"
  exit 0
fi

if [ "$MODE" = "wipe" ]; then
  echo "🧹 Running Rust wipe"
  cd "$BACKEND_DIR"
  LOCAL_DOCS_FOLDER="$LOCAL_DOCS_DIR" \
  LOCAL_INDEX_PATH="$LOCAL_DOCS_DIR/cards_index.json" \
  cargo run --bin wipe
  exit 0
fi

if [ "$MODE" != "dev" ]; then
  echo "❌ Unknown mode: $MODE"
  echo "   Usage: ./start-rust.sh [dev|local-parser|scraper|main-pipeline|wipe]"
  exit 1
fi

cleanup() {
  echo "🛑 Shutting down..."
  [ -n "${BACKEND_PID:-}" ] && kill "$BACKEND_PID" 2>/dev/null || true
  [ -n "${FRONTEND_PID:-}" ] && kill "$FRONTEND_PID" 2>/dev/null || true
}

trap cleanup SIGINT SIGTERM EXIT

run_backend

echo "⚛️  Starting Next.js Frontend against Rust API..."
cd "$FRONTEND_DIR"
if command -v yarn >/dev/null 2>&1; then
  NEXT_PUBLIC_API_URL="http://localhost:5002" yarn dev &
else
  NEXT_PUBLIC_API_URL="http://localhost:5002" npm run dev &
fi
FRONTEND_PID=$!

echo "✅ All systems go!"
echo "   Rust Backend: http://localhost:5002"
echo "   Frontend:     http://localhost:3000"
echo "   Press Ctrl+C to stop both servers."

wait
