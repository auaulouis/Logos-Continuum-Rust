# Logos Continuum (Local)

Quick links: [Screenshots](./screenshots/README.md)

This workspace runs Logos Continuum locally with:

- Frontend: Next.js app in `logos-web`
- Backend API + parser: Flask app in `verbatim-parser `

> Note: the backend folder name currently includes a trailing space: `verbatim-parser `.
> Always quote that path in shell commands.

## Known quirks

- Backend directory name has a trailing space: `verbatim-parser `
- Always quote backend paths in commands, for example: `cd "./verbatim-parser "`
- If you rename the folder to remove the trailing space, also update `start.sh`

## Requirements

- macOS/Linux shell
- Node.js 18 LTS (recommended)
- Python 3.9+

## Install

From the workspace root (`Logos backup`):

### 1) Backend install

```bash
cd "./verbatim-parser "
python3 -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
```

### 2) Frontend install

```bash
cd "./logos-web"
yarn install
```

If Yarn is unavailable, use npm:

```bash
npm install
```

## Run

### Start backend + frontend together

From workspace root:

```bash
./start.sh
```

Open:

- Frontend UI: http://localhost:3000
- Backend API: http://localhost:5001

### Start manually (optional)

> Reminder: keep backend paths quoted because of the trailing space in `verbatim-parser `.

Backend:

```bash
cd "./verbatim-parser "
source .venv/bin/activate
PORT=5001 python3 api.py
```

Frontend:

```bash
cd "./logos-web"
yarn dev
```

## Current UI usage

## Home page (`/`)

1. **Search box**: type a query and press **Search** to open the query page.
2. **Upload DOCX to parse now**:
	- Drag/drop `.docx` files or click to choose files
	- Files are uploaded to backend `local_docs/uploaded_docs`
	- Files are parsed immediately and indexed
	- Parsing output appears in the details box
3. **Parser Settings**: configure parser behavior used by API uploads/indexing:
	- `use_parallel_processing`
	- `parser_card_workers`
	- `local_parser_file_workers`
	- `flush_enabled`
	- `flush_every_docs`
4. **Manage Documents**:
	- View indexed + uploaded files
	- Search documents
	- Remove a document from index only
	- Delete uploaded file from folder
	- Bulk select + delete selected docs
5. **Clear Parsed Cards**:
	- Clear parsed cards from index
	- Optionally delete uploaded `.docx` files too

## Query page (`/query`)

1. **Search**: run term search from the top input.
   - Citation filter syntax is inline: `cite:<text>`
   - Example: `nuclear deterrence cite:brookings`
   - Citation-only search is also supported: `cite:harvard law review`
2. **Results list**:
	- Use **Tag Matches** and **Paragraph Matches** tabs
	- Use **Previous / page buttons / Next** for pagination
	- Click a result to open full card details
3. **Card actions**:
	- **Edit** the selected card
	- **Copy** card content
	- **Export Saved Edits (N)** exports saved edits to DOCX
	- **StyleSelect** changes copy/export styling
4. **Saved edits behavior**:
	- Card edits are persisted in browser `localStorage`
	- Export includes saved edits and resolved source document labels

## Data and local files

- Dev search index file: `verbatim-parser /local_docs_dev/cards_index.json`
- Dev uploaded docs folder: `verbatim-parser /local_docs_dev/uploaded_docs`
- Dev parser settings file: `verbatim-parser /local_docs_dev/parser_settings.json`

`./start.sh` now forces these dev-only paths so VS Code runs do not read/write desktop app data.

Desktop app persistence (Electron build):

- Cards/settings/docs are stored in `~/Documents/Logos Continuum/local_docs`
- On first launch after this change, existing data from Electron `userData/local_docs` is copied into that folder
- This keeps parsed cards and parser settings available across app updates/reinstalls (as long as `~/Documents/Logos Continuum` is kept)
- Desktop backend runs on `http://127.0.0.1:5501` (separate from dev backend `:5001`) to prevent accidental cross-attachment

When the backend starts and the index is empty, it auto-indexes local `.docx` files under `local_docs`.

## Useful backend commands

From `verbatim-parser ` with `.venv` active:

```bash
# Parse local docs in batch
python3 local_parser.py

# Profile parser + set card workers
PARSER_PROFILE=1 PARSER_CARD_WORKERS=4 python3 local_parser.py

# Clear local index file
python3 wipe.py
```

## Troubleshooting

### `yarn dev` fails in `logos-web`

```bash
cd "./logos-web"
rm -rf .next
yarn install
yarn dev
```

If it still fails, verify Node version (recommended: Node 18 LTS):

```bash
node -v
```

### Port already in use (`3000` or `5001`)

Check and stop processes using those ports:

```bash
lsof -i :3000
lsof -i :5001
kill -9 <PID>
```

Then restart:

```bash
./start.sh
```

### Python environment mismatch

This repo may have multiple virtual environments (`/Logos backup/.venv` and `verbatim-parser /.venv`).
`start.sh` expects the backend environment at `verbatim-parser /.venv`.

Recreate backend venv if missing/broken:

```bash
cd "./verbatim-parser "
python3 -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
```

### Backend starts but search is empty

- Upload `.docx` files from the Home page, or place docs under `verbatim-parser /local_docs`
- The backend auto-indexes local docs when `cards_index.json` is empty
- You can clear and rebuild with:

```bash
cd "./verbatim-parser "
source .venv/bin/activate
python3 wipe.py
PORT=5001 python3 api.py
```

## Credits

Based on [tvergho/logos-web](https://github.com/tvergho/logos-web), adapted for local/offline use.

## Licence

The MIT License (MIT)
Copyright © 2026 auaulouis

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the “Software”), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED “AS IS”, WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.