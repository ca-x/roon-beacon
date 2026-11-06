import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { CancelledError, useQuery, useQueryClient } from '@tanstack/react-query';
import * as Dialog from '@radix-ui/react-dialog';
import { Radio } from 'lucide-react';
import { BrowserAudio } from './BrowserAudio';
import { LibraryPanel } from './Library';
import { PlayerPanel } from './Player';
import { Settings } from './Settings';
import { I18n, Button, Notice, TextField, type Translate } from './ui';
import { copy } from './i18n';
import { errorMessage, request } from './api';
import type { AudioStatus, Auth, Player, Status } from './types';

function initialLanguage(): 'en' | 'zh' {
  try {
    const saved = localStorage.getItem('roonBeaconLang');
    if (saved === 'en' || saved === 'zh') return saved;
  } catch {}
  return navigator.language.startsWith('zh') ? 'zh' : 'en';
}

type Theme = 'light' | 'dark' | 'system';

function initialTheme(): Theme {
  try {
    const saved = localStorage.getItem('roonBeaconTheme');
    if (saved === 'light' || saved === 'dark') return saved;
  } catch {}
  return 'system';
}

export function App() {
  const queryClient = useQueryClient();
  const [lang, setLang] = useState(initialLanguage);
  const [theme, setTheme] = useState(initialTheme);
  const [epoch, setEpoch] = useState(0);
  const revision = useRef(0);
  const [session] = useState(() => `beacon-${Array.from(crypto.getRandomValues(new Uint8Array(16)), (byte) => byte.toString(16).padStart(2, '0')).join('')}`);
  const [zoneId, setZoneId] = useState('');
  const [selecting, setSelecting] = useState(false);
  const [appError, setAppError] = useState('');
  const [dialogOpen, setDialogOpen] = useState(false);
  const [token, setToken] = useState('');
  const [loginError, setLoginError] = useState('');
  const [loggingIn, setLoggingIn] = useState(false);
  const [desktopToken] = useState(() => {
    const value = new URLSearchParams(location.hash.slice(1)).get('desktop-token');
    return value && /^[a-f0-9]{64}$/.test(value) ? value : '';
  });
  const [bootstrapping, setBootstrapping] = useState(Boolean(desktopToken));
  const main = useRef<HTMLElement>(null);
  const authButton = useRef<HTMLButtonElement>(null);
  const text: Translate = (key, values = {}) => {
    const dictionary: Record<string, string> = copy[lang];
    const english: Record<string, string> = copy.en;
    let value = dictionary[key] || english[key] || key;
    Object.entries(values).forEach(([name, replacement]) => { value = value.replaceAll(`{${name}}`, String(replacement)); });
    return value;
  };
  useEffect(() => {
    document.documentElement.lang = lang === 'zh' ? 'zh-CN' : 'en';
    try { localStorage.setItem('roonBeaconLang', lang); } catch {}
  }, [lang]);
  useEffect(() => {
    const media = matchMedia('(prefers-color-scheme: dark)');
    const apply = () => {
      const resolved = theme === 'system' ? media.matches ? 'dark' : 'light' : theme;
      document.documentElement.dataset.theme = resolved;
      document.documentElement.style.colorScheme = resolved;
      document.querySelector<HTMLMetaElement>('meta[name="theme-color"]')?.setAttribute('content', resolved === 'dark' ? '#191918' : '#f7f5f1');
    };
    apply();
    try { localStorage.setItem('roonBeaconTheme', theme); } catch {}
    media.addEventListener('change', apply);
    return () => media.removeEventListener('change', apply);
  }, [theme]);
  useEffect(() => {
    const pointer = () => { document.documentElement.dataset.input = 'pointer'; };
    const keyboard = () => { document.documentElement.dataset.input = 'keyboard'; };
    document.addEventListener('pointerdown', pointer, true);
    document.addEventListener('keydown', keyboard, true);
    return () => { document.removeEventListener('pointerdown', pointer, true); document.removeEventListener('keydown', keyboard, true); };
  }, []);

  const auth = useQuery({ queryKey: ['auth'], enabled: !bootstrapping, queryFn: ({ signal }) => request<Auth>('/api/auth', undefined, signal), refetchInterval: 6000 });
  useEffect(() => {
    if (!desktopToken) return;
    history.replaceState(null, '', location.pathname + location.search);
    request('/api/login', { token: desktopToken }).catch((failure) => setAppError(errorMessage(failure))).finally(() => setBootstrapping(false));
  }, [desktopToken]);
  const locked = !auth.data || (auth.data.required && !auth.data.authenticated);
  useLayoutEffect(() => { if (main.current) main.current.inert = locked; }, [locked]);
  useEffect(() => { if (auth.data?.required && !auth.data.authenticated) setDialogOpen(true); }, [auth.data]);
  useEffect(() => {
    const expire = () => {
      revision.current += 1;
      setEpoch(revision.current);
      setSelecting(false);
      void queryClient.cancelQueries({ queryKey: ['player'] });
      queryClient.setQueryData<Auth>(['auth'], { required: true, authenticated: false });
      setAppError(text('authExpired'));
    };
    window.addEventListener('beacon-auth-expired', expire);
    return () => window.removeEventListener('beacon-auth-expired', expire);
  }, [queryClient, lang]);

  const player = useQuery({
    queryKey: ['player', epoch], enabled: !locked && !selecting, refetchInterval: 2000,
    queryFn: async ({ signal }) => {
      const expected = revision.current;
      const data = await request<Player>('/api/player', undefined, signal);
      if (expected !== revision.current) throw new CancelledError();
      if (!Array.isArray(data.cores) || !Array.isArray(data.zones)) throw new Error(text('invalidResponse'));
      return data;
    }
  });
  const status = useQuery({ queryKey: ['status', epoch], enabled: !locked, queryFn: ({ signal }) => request<Status>('/api/status', undefined, signal), refetchInterval: 6000 });
  const audio = useQuery({ queryKey: ['audio', epoch], enabled: !locked, queryFn: ({ signal }) => request<AudioStatus>('/api/audio/status', undefined, signal), refetchInterval: 6000 });
  const coreId = player.data?.selected_core_id || null;
  const native = Boolean(desktopToken) || audio.data?.mode === 'native';
  const core = player.data?.cores.find((candidate) => candidate.core_id === coreId);
  const zones = player.data?.zones || [];
  const zone = zones.find((candidate) => candidate.zone_id === zoneId) || zones[0];
  const ready = !locked && !selecting && !player.isError && core?.connection === 'connected';
  useEffect(() => { setZoneId(''); }, [coreId]);
  const refreshPlayer = async () => { await player.refetch(); };
  const connection = player.isError ? 'offline' : core ? core.connection : player.data?.cores.length ? 'chooseCore' : 'looking';

  const changeDialog = (open: boolean) => {
    setDialogOpen(open);
    if (!open) { setToken(''); setLoginError(''); }
  };
  return <I18n.Provider value={text}>
    <a className="skip-link" href="#main">{text('skip')}</a>
    <div className="shell">
      <header className="masthead"><a className="brand" href="/" aria-label="Roon Beacon"><img className="brand-logo" src="/assets/logo.png" alt="Roon Beacon" width={60} height={60} /><span>Roon <strong>Beacon</strong><small>{text(native ? 'desktopSubtitle' : 'subtitle')}</small></span></a>
        <div className="header-actions"><div className="language" role="group" aria-label="Language / 语言"><Button data-lang="en" lang="en" aria-pressed={lang === 'en'} onClick={() => setLang('en')}>EN</Button><Button data-lang="zh" lang="zh-CN" aria-pressed={lang === 'zh'} onClick={() => setLang('zh')}>中文</Button></div>
          <label className="theme-choice"><span className="sr-only">{lang === 'zh' ? '主题' : 'Theme'}</span><select id="themeSelect" value={theme} onChange={(event) => setTheme(event.target.value as Theme)}><option value="light">{lang === 'zh' ? '浅色' : 'Light'}</option><option value="dark">{lang === 'zh' ? '深色' : 'Dark'}</option><option value="system">{lang === 'zh' ? '跟随系统' : 'System'}</option></select></label>
          <Button ref={authButton} id="authButton" className="text-button" hidden={!auth.data?.required} onClick={async () => {
            if (locked) { setDialogOpen(true); return; }
            try {
              await request('/api/logout', {});
              window.dispatchEvent(new Event('beacon-auth-expired'));
              setAppError('');
            } catch (failure) { setAppError(errorMessage(failure)); }
          }}>{text(locked ? 'unlock' : 'lock')}</Button>
        </div>
      </header>
      <Notice id="appError" error={appError || errorMessage(auth.error) || errorMessage(player.error) || player.data?.discovery_error || core?.error} />
      <p id="lockNotice" className="notice" hidden={!locked}>{text('locked')}</p>
      <main id="main" ref={main} tabIndex={-1}>
        <section className="connection-bar" aria-label={text('connection')}>
          <label className="selector"><span>{text('core')}</span><select id="coreSelect" value={coreId || ''} disabled={locked || selecting || !player.data?.cores.length} onChange={async (event) => {
            const nextCore = event.target.value;
            if (!nextCore) return;
            setSelecting(true);
            revision.current += 1;
            const expected = revision.current;
            setEpoch(expected);
            await queryClient.cancelQueries({ queryKey: ['player'] });
            try {
              await request('/api/player/core', { core_id: nextCore });
              if (expected === revision.current) { setZoneId(''); setAppError(''); }
            }
            catch (failure) { if (expected === revision.current) setAppError(errorMessage(failure)); }
            finally { if (expected === revision.current) setSelecting(false); }
          }}>
            {!coreId && <option value="">{text(player.data?.cores.length ? 'chooseCore' : 'noCores')}</option>}
            {player.data?.cores.map((candidate) => <option key={candidate.core_id} value={candidate.core_id}>{candidate.name || candidate.host} · {text(candidate.connection)}</option>)}
          </select></label>
          <label className="selector"><span>{text('zone')}</span><select id="zoneSelect" value={zone?.zone_id || ''} disabled={!ready || !zones.length} onChange={(event) => setZoneId(event.target.value)}>
            {!zones.length && <option value="">{text('noZones')}</option>}{zones.map((candidate) => <option key={candidate.zone_id} value={candidate.zone_id}>{candidate.display_name}</option>)}
          </select></label>
          <span id="connectionState" className="connection-state" role="status">{text(connection)}</span>
        </section>
        <div className="music-layout">
          <LibraryPanel key={`${epoch}:${coreId}:${core?.connection}`} ready={ready} zoneId={zone?.zone_id || ''} session={session} />
          <aside className="listening" aria-label={text('nowPlaying')}><PlayerPanel coreId={coreId} zone={zone} zones={zones} outputs={player.data?.outputs || []} ready={ready} refresh={refreshPlayer} /><BrowserAudio status={audio.isError ? undefined : audio.data} locked={locked} statusError={errorMessage(audio.error)} /></aside>
        </div>
        <Settings native={native} status={status.data} statusError={errorMessage(status.error)} locked={locked} refresh={async () => { await status.refetch(); }} />
      </main>
    </div>
    <Dialog.Root open={dialogOpen} onOpenChange={changeDialog}><Dialog.Portal><Dialog.Overlay className="dialog-overlay" /><Dialog.Content id="loginDialog" className="login-dialog" aria-labelledby="loginHeading" aria-describedby="loginHint" onCloseAutoFocus={(event) => { event.preventDefault(); authButton.current?.focus(); }}>
      <form id="loginForm" onSubmit={async (event) => {
        event.preventDefault(); setLoggingIn(true); setLoginError('');
        try {
          await request('/api/login', { token });
          setToken('');
          const result = await auth.refetch();
          if (result.data && (!result.data.required || result.data.authenticated)) { changeDialog(false); setAppError(''); }
        } catch (failure) { setLoginError(errorMessage(failure)); } finally { setLoggingIn(false); }
      }}>
        <Radio className="login-icon" aria-hidden="true" /><Dialog.Title id="loginHeading">{text('loginHeading')}</Dialog.Title><Dialog.Description id="loginHint" className="small-note">{text('loginHint')}</Dialog.Description>
        <TextField id="accessToken" name="token" variant="outline" label={text('accessToken')} type="password" autoComplete="current-password" value={token} onChange={(event) => setToken(event.target.value)} required aria-describedby="loginError" />
        <Notice id="loginError" error={loginError} /><div className="inline-actions"><Button id="loginSubmit" className="primary" type="submit" disabled={loggingIn}>{text('unlock')}</Button><Dialog.Close asChild><Button id="loginCancel" className="text-button">{text('cancel')}</Button></Dialog.Close></div>
      </form>
    </Dialog.Content></Dialog.Portal></Dialog.Root>
  </I18n.Provider>;
}
