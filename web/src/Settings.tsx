import { useEffect, useRef, useState } from 'react';
import { SlidersHorizontal } from 'lucide-react';
import { Button, Notice, TextField, useText } from './ui';
import { clockTime, errorMessage, request } from './api';
import type { RuntimeConfig, Status } from './types';
import { AudioNetworkSettings } from './AudioNetworkSettings';

export function Settings({ status, statusError, locked, refresh }: { status?: Status; statusError?: string; locked: boolean; refresh: () => Promise<unknown> }) {
  const text = useText();
  const [config, setConfig] = useState<RuntimeConfig>({ name: '', broadcast_interval_secs: 0, live_proxy_timeout_ms: 500, reverse_proxy: false, proxy_ip: null });
  const [interval, setIntervalValue] = useState('0');
  const [timeout, setTimeoutValue] = useState('500');
  const [error, setError] = useState('');
  const [feedback, setFeedback] = useState('');
  const [saving, setSaving] = useState(false);
  const dirty = useRef(false);
  useEffect(() => {
    if (status?.config && !dirty.current) {
      setConfig(status.config);
      setIntervalValue(String(status.config.broadcast_interval_secs));
      setTimeoutValue(String(status.config.live_proxy_timeout_ms));
    }
  }, [status]);
  const mode = (status?.forwarding_enabled ?? Boolean(status?.remote_ip)) ? 'forwarding' : 'standalone';
  return <>
    <details id="settings" className="settings"><summary><SlidersHorizontal aria-hidden="true" /><span>{text('settings')}</span><span className="settings-meta" id="settingsMode">{text(mode)}</span></summary>
      <div className="settings-body">
        <section aria-labelledby="signalHeading"><h2 id="signalHeading">{text('signal')}</h2>
          <dl className="signal-path"><div><dt>{text('local')}</dt><dd id="localIp">{status?.local_ip || '—'}</dd></div><div><dt>{text('remote')}</dt><dd id="remoteIp">{status?.remote_ip || text('standalone')}</dd></div><div><dt>{text('ports')}</dt><dd id="localPorts">{status?.sood_port ? `SOOD ${status.sood_port} · TCP ${status.discovery_port ?? '—'}` : '—'}</dd></div><div><dt>{text('mode')}</dt><dd id="mode">{text(status?.use_broadcast ? 'broadcast' : 'multicast')}</dd></div></dl>
          <dl className="metrics"><div><dt>{text('queries')}</dt><dd id="q">{status?.metrics?.sood_queries ?? '—'}</dd></div><div><dt>{text('live')}</dt><dd id="live">{status?.metrics?.live_core_responses ?? '—'}</dd></div><div><dt>{text('fallback')}</dt><dd id="fallback">{status?.metrics?.template_fallbacks ?? '—'}</dd></div><div><dt>{text('tcp')}</dt><dd id="tcp">{status?.metrics?.tcp_connections ?? '—'}</dd></div></dl>
          <Notice id="statusError" error={statusError} />
        </section>
        <form id="configForm" onChange={() => { dirty.current = true; setFeedback(''); }} onSubmit={async (event) => {
          event.preventDefault();
          setSaving(true); setError(''); setFeedback(text('saving'));
          try {
            await request('/api/config', { ...config, broadcast_interval_secs: Number(interval), live_proxy_timeout_ms: Number(timeout), proxy_ip: config.proxy_ip?.trim() || null });
            dirty.current = false;
            await refresh();
            setFeedback(text('saved'));
          } catch (failure) { setError(errorMessage(failure)); setFeedback(''); } finally { setSaving(false); }
        }}>
          <h2>{text('runtime')}</h2>
          <TextField id="name" variant="outline" label={text('name')} value={config.name} onChange={(event) => setConfig({ ...config, name: event.target.value })} autoComplete="off" required maxLength={200} disabled={locked} />
          <div className="form-columns"><TextField id="broadcastInterval" variant="outline" label={text('interval')} type="number" min={0} max={3600} required value={interval} disabled={locked} onChange={(event) => setIntervalValue(event.target.value)} /><TextField id="liveTimeout" variant="outline" label={text('timeout')} type="number" min={100} max={10000} required value={timeout} disabled={locked} onChange={(event) => setTimeoutValue(event.target.value)} /></div>
          <label className="checkbox-field"><input id="reverseProxy" type="checkbox" checked={config.reverse_proxy} disabled={locked} onChange={(event) => setConfig({ ...config, reverse_proxy: event.target.checked })} /><span>{text('reverse')}</span></label>
          <TextField id="proxyIp" variant="outline" label={text('proxyIp')} value={config.proxy_ip || ''} disabled={locked} autoComplete="off" onChange={(event) => setConfig({ ...config, proxy_ip: event.target.value })} />
          <div className="inline-actions"><Button id="configSave" className="secondary" type="submit" disabled={saving || locked}>{text('save')}</Button><span id="feedback" className="small-note" role="status">{feedback}</span></div>
          <Notice id="configError" error={error} />
        </form>
        <AudioNetworkSettings locked={locked} refresh={refresh} />
      </div>
    </details>
    <footer><span>{text('footer')}</span><span><span id="version">{status?.version ? `v${status.version}` : ''}</span>{text('uptime')} <span id="uptime">{status ? clockTime(status.uptime_secs) : '—'}</span></span></footer>
  </>;
}
