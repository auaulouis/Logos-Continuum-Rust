/* eslint-disable jsx-a11y/anchor-is-valid */
import '../styles/globals.scss';
import 'react-date-range/dist/styles.css'; // main style file
import 'react-date-range/dist/theme/default.css'; // theme css file
import { useEffect, useMemo, useState } from 'react';
import { SessionProvider } from 'next-auth/react';
import mixpanel from 'mixpanel-browser';
import type { AppProps } from 'next/app';
import { AppContext, defaultState } from '../lib/appContext';
import { ensureDesktopBackendRunning } from '../lib/desktopRuntime';

mixpanel.init(process.env.NEXT_PUBLIC_MIXPANEL_KEY || '');

function MyApp({ Component, pageProps: { session, ...pageProps } }: AppProps) {
  const [highlightColor, setHighlightColor] = useState<string>(defaultState.highlightColor);
  const [theme, setTheme] = useState(defaultState.theme);

  useEffect(() => {
    if (typeof window === 'undefined') {
      return;
    }

    const savedTheme = window.localStorage.getItem('logos-theme');
    if (savedTheme === 'light' || savedTheme === 'dark') {
      setTheme(savedTheme);
      return;
    }

    if (window.matchMedia('(prefers-color-scheme: dark)').matches) {
      setTheme('dark');
    }
  }, []);

  useEffect(() => {
    ensureDesktopBackendRunning().catch((error) => {
      console.error('Failed to ensure desktop backend is running:', error);
    });
  }, []);

  useEffect(() => {
    if (typeof document === 'undefined') {
      return;
    }
    document.documentElement.dataset.theme = theme;
    document.body.dataset.theme = theme;
    window.localStorage.setItem('logos-theme', theme);
  }, [theme]);

  const state = useMemo(() => {
    return {
      highlightColor,
      setHighlightColor,
      theme,
      setTheme,
      toggleTheme: () => setTheme((previousTheme) => (previousTheme === 'light' ? 'dark' : 'light')),
    };
  }, [highlightColor, theme]);

  return (
    <SessionProvider session={session}>
      <AppContext.Provider value={state}>
        <Component {...pageProps} />
      </AppContext.Provider>
    </SessionProvider>
  );
}

export default MyApp;
