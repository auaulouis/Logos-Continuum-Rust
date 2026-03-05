const { app, BrowserWindow } = require('electron');
const path = require('path');
const http = require('http');
const net = require('net');
const fs = require('fs');
const { spawn } = require('child_process');
const next = require('next');

let mainWindow;
let nextServer;
let backendProcess;
const APP_BACKEND_PORT = 5501;

function ensureDir(dirPath) {
  fs.mkdirSync(dirPath, { recursive: true });
}

function copyDirectoryMissingOnly(sourceDir, destinationDir) {
  if (!fs.existsSync(sourceDir)) {
    return;
  }

  ensureDir(destinationDir);
  const entries = fs.readdirSync(sourceDir, { withFileTypes: true });

  for (const entry of entries) {
    const sourcePath = path.join(sourceDir, entry.name);
    const destinationPath = path.join(destinationDir, entry.name);

    if (entry.isDirectory()) {
      copyDirectoryMissingOnly(sourcePath, destinationPath);
      continue;
    }

    if (!entry.isFile()) {
      continue;
    }

    if (!fs.existsSync(destinationPath)) {
      fs.copyFileSync(sourcePath, destinationPath);
    }
  }
}

function resolvePersistentLocalDocsDir() {
  try {
    const documentsDir = app.getPath('documents');
    return path.join(documentsDir, 'Logos Continuum', 'local_docs');
  } catch {
    return path.join(app.getPath('userData'), 'local_docs');
  }
}

function preparePersistentLocalDocsDir() {
  const persistentLocalDocsDir = resolvePersistentLocalDocsDir();
  const legacyLocalDocsDir = path.join(app.getPath('userData'), 'local_docs');

  ensureDir(persistentLocalDocsDir);

  if (legacyLocalDocsDir !== persistentLocalDocsDir && fs.existsSync(legacyLocalDocsDir)) {
    copyDirectoryMissingOnly(legacyLocalDocsDir, persistentLocalDocsDir);
  }

  return persistentLocalDocsDir;
}

function getBackendRoot() {
  if (app.isPackaged) {
    return path.join(process.resourcesPath, 'backend');
  }

  return path.resolve(getAppDir(), '..', 'rust-backend', 'target', 'release');
}

function resolveBackendExecutable(backendRoot) {
  const binaryName = process.platform === 'win32' ? 'logos-backend.exe' : 'logos-backend';
  return path.join(backendRoot, binaryName);
}

function isPortOpen(port) {
  return new Promise((resolve) => {
    const socket = net.createConnection({ port, host: '127.0.0.1' });
    socket.once('connect', () => {
      socket.end();
      resolve(true);
    });
    socket.once('error', () => resolve(false));
  });
}

function waitForBackend(port, timeoutMs = 20000) {
  const start = Date.now();

  return new Promise((resolve, reject) => {
    const tick = async () => {
      const open = await isPortOpen(port);
      if (open) {
        resolve(true);
        return;
      }

      if (Date.now() - start > timeoutMs) {
        reject(new Error(`Parser backend did not start on port ${port} in time`));
        return;
      }

      setTimeout(tick, 300);
    };

    tick();
  });
}

function isCompatibleBackendRunning(port, timeoutMs = 2000) {
  return new Promise((resolve) => {
    const req = http.get(
      {
        host: '127.0.0.1',
        port,
        path: '/parser-settings',
      },
      (res) => {
        const status = Number(res.statusCode || 0);
        res.resume();
        resolve(status >= 200 && status < 500);
      },
    );

    req.setTimeout(timeoutMs, () => {
      req.destroy();
      resolve(false);
    });

    req.on('error', () => resolve(false));
  });
}

async function startParserBackend() {
  const portInUse = await isPortOpen(APP_BACKEND_PORT);
  if (portInUse) {
    const compatibleBackend = await isCompatibleBackendRunning(APP_BACKEND_PORT);
    if (compatibleBackend) {
      console.warn(`Desktop backend port ${APP_BACKEND_PORT} already has a compatible backend; reusing it`);
      return;
    }

    throw new Error(`Desktop backend port ${APP_BACKEND_PORT} is already in use by an incompatible process`);
  }

  const backendRoot = getBackendRoot();
  const backendExecutable = resolveBackendExecutable(backendRoot);

  if (!fs.existsSync(backendExecutable)) {
    throw new Error(`Rust backend binary not found at ${backendExecutable}`);
  }

  if (process.platform !== 'win32') {
    fs.chmodSync(backendExecutable, 0o755);
  }

  const localDocsDir = preparePersistentLocalDocsDir();

  const env = {
    ...process.env,
    LOCAL_DOCS_FOLDER: localDocsDir,
    LOCAL_INDEX_PATH: path.join(localDocsDir, 'cards_index.json'),
    CARD_ID_REGISTRY_PATH: path.join(localDocsDir, 'card_id_registry.json'),
    PARSER_SETTINGS_PATH: path.join(localDocsDir, 'parser_settings.json'),
    PARSER_EVENTS_PATH: path.join(localDocsDir, 'parser_events.jsonl'),
    PORT: String(APP_BACKEND_PORT),
  };

  backendProcess = spawn(backendExecutable, {
    cwd: path.dirname(backendExecutable),
    env,
    stdio: 'inherit',
  });

  backendProcess.on('exit', (code, signal) => {
    if (!app.isQuitting) {
      console.error(`Parser backend exited unexpectedly (code=${code}, signal=${signal})`);
    }
  });

  await waitForBackend(APP_BACKEND_PORT);
}

function getAppDir() {
  return app.getAppPath();
}

function waitForPort(port) {
  return new Promise((resolve) => {
    const socket = net.createConnection({ port, host: '127.0.0.1' });
    socket.once('connect', () => {
      socket.end();
      resolve(true);
    });
    socket.once('error', () => resolve(false));
  });
}

async function findOpenPort(start = 3001, max = 3015) {
  for (let port = start; port <= max; port += 1) {
    const inUse = await waitForPort(port);
    if (!inUse) {
      return port;
    }
  }

  return 0;
}

async function startNextServer() {
  const appDir = getAppDir();
  const nextApp = next({
    dev: false,
    dir: appDir,
    conf: {
      distDir: '.next',
    },
  });

  const handle = nextApp.getRequestHandler();
  await nextApp.prepare();

  const port = await findOpenPort();
  if (!port) {
    throw new Error('No available port between 3001 and 3015');
  }

  await new Promise((resolve, reject) => {
    nextServer = http.createServer((req, res) => handle(req, res));
    nextServer.once('error', reject);
    nextServer.listen(port, '127.0.0.1', resolve);
  });

  return `http://127.0.0.1:${port}`;
}

async function createWindow() {
  await startParserBackend();
  const appUrl = await startNextServer();
  const isMac = process.platform === 'darwin';

  mainWindow = new BrowserWindow({
    width: 1360,
    height: 860,
    minWidth: 1080,
    minHeight: 720,
    title: '',
    autoHideMenuBar: true,
    titleBarStyle: isMac ? 'default' : undefined,
    vibrancy: isMac ? 'titlebar' : undefined,
    visualEffectState: isMac ? 'active' : undefined,
    backgroundColor: '#111111',
    webPreferences: {
      contextIsolation: true,
      nodeIntegration: false,
    },
  });

  await mainWindow.loadURL(appUrl);
}

app.whenReady().then(createWindow).catch((error) => {
  console.error('Failed to start desktop app:', error);
  app.quit();
});

app.on('window-all-closed', () => {
  app.isQuitting = true;
  if (nextServer) {
    nextServer.close();
  }

  if (backendProcess && !backendProcess.killed) {
    backendProcess.kill('SIGTERM');
  }

  if (process.platform !== 'darwin') {
    app.quit();
  }
});

app.on('activate', () => {
  if (BrowserWindow.getAllWindows().length === 0) {
    createWindow();
  }
});
