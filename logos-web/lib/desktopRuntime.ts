type TauriInvoke = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;

let backendStartPromise: Promise<void> | null = null;

export const isElectronRuntime = () => (
  typeof navigator !== 'undefined' && navigator.userAgent.includes('Electron')
);

export const isTauriRuntime = () => (
  typeof navigator !== 'undefined' && navigator.userAgent.includes('Tauri')
);

export const isDesktopRuntime = () => isElectronRuntime() || isTauriRuntime();

async function getTauriInvoke(): Promise<TauriInvoke | null> {
  if (!isTauriRuntime()) {
    return null;
  }

  const api = await import('@tauri-apps/api/core');
  return api.invoke as TauriInvoke;
}

export async function ensureDesktopBackendRunning(): Promise<void> {
  if (!isDesktopRuntime()) {
    return;
  }

  if (isElectronRuntime()) {
    return;
  }

  if (backendStartPromise) {
    return backendStartPromise;
  }

  backendStartPromise = (async () => {
    const invoke = await getTauriInvoke();
    if (!invoke) {
      throw new Error('Tauri invoke API is unavailable in this renderer');
    }

    await invoke<number>('start_backend');
  })();

  return backendStartPromise;
}

export async function getDesktopAppVersion(): Promise<string | null> {
  if (!isTauriRuntime()) {
    return null;
  }

  const invoke = await getTauriInvoke();
  if (!invoke) {
    return null;
  }

  return invoke<string>('app_version');
}

export async function invokeTauriCommand<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const invoke = await getTauriInvoke();
  if (!invoke) {
    throw new Error('Tauri invoke API is unavailable in this renderer');
  }

  return invoke<T>(cmd, args);
}
