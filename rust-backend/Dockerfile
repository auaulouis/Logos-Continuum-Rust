FROM --platform=linux/amd64 rust:1.78-slim-bookworm AS builder

WORKDIR /build
COPY . .

RUN cargo build --release --bin logos-backend

FROM --platform=linux/amd64 debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app
RUN mkdir -p /app/local_docs/uploaded_docs

COPY --from=builder /build/target/release/logos-backend /usr/local/bin/logos-backend

ENV LOCAL_DOCS_FOLDER=/app/local_docs
ENV LOCAL_INDEX_PATH=/app/local_docs/cards_index.json
ENV CARD_ID_REGISTRY_PATH=/app/local_docs/card_id_registry.json
ENV PARSER_SETTINGS_PATH=/app/local_docs/parser_settings.json
ENV PARSER_EVENTS_PATH=/app/local_docs/parser_events.jsonl
ENV PORT=5002

EXPOSE 5002

CMD ["logos-backend"]