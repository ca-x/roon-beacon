import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Button, Notice, useText } from './ui';
import { errorMessage, request } from './api';
import type { AudioNetwork } from './types';

export function AudioNetworkSettings({ locked, refresh }: { locked: boolean; refresh: () => Promise<unknown> }) {
  const text = useText();
  const network = useQuery({ queryKey: ['audio-network'], enabled: !locked, refetchInterval: 5000,
    queryFn: ({ signal }) => request<AudioNetwork>('/api/audio/network', undefined, signal) });
  const [editing, setEditing] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [saved, setSaved] = useState(false);
  const selected = editing ?? network.data?.interface ?? '';
  const missing = Boolean(selected && network.data && !network.data.interfaces.some((item) => item.name === selected));
  return <form className="audio-network-settings" aria-labelledby="audioNetworkHeading" onSubmit={async (event) => {
    event.preventDefault();
    setSaving(true); setError(''); setSaved(false);
    try {
      await request('/api/audio/network', { interface: selected || null });
      await network.refetch();
      setEditing(null); setSaved(true);
      await refresh();
    } catch (failure) { setError(errorMessage(failure)); } finally { setSaving(false); }
  }}>
    <h2 id="audioNetworkHeading">{text('audioNetwork')}</h2>
    <p id="audioNetworkHint" className="small-note">{text('audioNetworkHint')}</p>
    <label className="field" htmlFor="audioNetworkSelect"><span>{text('audioInterface')}</span>
      <select id="audioNetworkSelect" value={selected} disabled={locked || saving || !network.data} aria-describedby="audioNetworkHint" onChange={(event) => {
        setEditing(event.target.value); setSaved(false); setError('');
      }}>
        <option value="">{text('audioNetworkAuto')}</option>
        {missing && <option value={selected}>{selected} · {text('audioNetworkMissing')}</option>}
        {network.data?.interfaces.map((item) => <option key={item.name} value={item.name}>{item.name} · {item.addresses.join(' / ')}</option>)}
      </select>
    </label>
    {missing && <p className="small-note">{text('audioNetworkMissingHint')}</p>}
    <div className="inline-actions"><Button id="audioNetworkSave" type="submit" className="primary" disabled={locked || saving || !network.data || missing}>{text(saving ? 'saving' : 'saveAudioNetwork')}</Button>
      <span id="audioNetworkFeedback" className="form-feedback" role="status">{saved ? text('audioNetworkSaved') : ''}</span></div>
    <Notice id="audioNetworkError" error={error || (network.isError ? errorMessage(network.error) : '')} />
  </form>;
}
