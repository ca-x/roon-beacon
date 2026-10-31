import { useCallback, useEffect, useRef, useState } from 'react';
import { Volume2 } from 'lucide-react';
import { Button, Notice, useText } from './ui';
import { errorMessage } from './api';
import type { AudioStatus } from './types';

export function BrowserAudio({ status, locked, statusError }: { status?: AudioStatus; locked: boolean; statusError?: string }) {
  const text = useText();
  const audio = useRef<HTMLAudioElement>(null);
  const joinedRef = useRef(false);
  const attempt = useRef(0);
  const [joined, setJoined] = useState(false);
  const [phase, setPhase] = useState('audioJoining');
  const [error, setError] = useState('');
  const [loudness, setLoudness] = useState(1);
  const supported = Boolean(document.createElement('audio').canPlayType('audio/mpeg'));
  const leave = useCallback(() => {
    attempt.current += 1;
    joinedRef.current = false;
    setJoined(false);
    if (audio.current) {
      audio.current.pause();
      audio.current.removeAttribute('src');
      audio.current.load();
    }
  }, []);
  useEffect(() => {
    if (locked || !status?.enabled || !status.available || status.state === 'error') {
      if (joinedRef.current) leave();
    }
  }, [locked, status, leave]);
  useEffect(() => {
    window.addEventListener('pagehide', leave);
    return () => { window.removeEventListener('pagehide', leave); leave(); };
  }, [leave]);

  const join = () => {
    if (!audio.current || joinedRef.current) return;
    const currentAttempt = ++attempt.current;
    joinedRef.current = true;
    setJoined(true);
    setPhase('audioJoining');
    setError('');
    audio.current.src = '/api/audio/stream';
    try {
      const playback = audio.current.play();
      playback.catch(() => {
        if (!joinedRef.current || attempt.current !== currentAttempt) return;
        leave();
        setError(text('audioFailed'));
      });
    } catch (failure) { leave(); setError(errorMessage(failure)); }
  };
  let message: string;
  if (!status) message = text('audioChecking');
  else if (!status.enabled || status.state === 'disabled') message = text('audioDisabled');
  else if (!supported) message = text('audioFormat');
  else if (!status.available || status.state === 'error') message = text('audioUnavailable');
  else if (joined && status.state === 'waiting') message = text('audioJoinedWaiting', { name: status.name || 'Beacon' });
  else if (joined) message = text(phase, { name: status.name || 'Beacon' });
  else if (status.state === 'starting') message = text('audioStarting');
  else message = text(status.state === 'streaming' ? 'audioStreaming' : 'audioWaiting', { name: status.name || 'Beacon', count: status.listeners || 0 });
  return <section className="browser-listening" aria-labelledby="browserHeading">
    <div className="audio-heading"><h2 id="browserHeading">{text('browserListening')}</h2><span className="experimental">{text('experimental')}</span></div>
    <p id="audioStatus" className="small-note" role="status">{message}</p><p className="small-note">{text('audioExplanation')}</p>
    <div className="inline-actions"><Button id="audioJoin" className="secondary" disabled={locked || joined || !status?.enabled || !status.available || ['error', 'starting', 'disabled'].includes(status.state) || !supported} onClick={join}>{text('join')}</Button><Button id="audioLeave" className="text-button" disabled={!joined} onClick={leave}>{text('leave')}</Button></div>
    <div id="browserVolumeControl" className="browser-volume" hidden={!joined}><label htmlFor="browserVolume">{text('browserVolume')}</label><div className="volume-line"><Volume2 aria-hidden="true" /><input id="browserVolume" type="range" min={0} max={1} step={0.01} value={loudness} onChange={(event) => {
      if (!audio.current) return;
      try { audio.current.volume = Number(event.target.value); } catch {}
      setLoudness(audio.current.volume);
    }} /><output id="browserVolumeValue" htmlFor="browserVolume">{Math.round(loudness * 100)}%</output></div><p className="small-note">{text('browserVolumeHint')}</p></div>
    <Notice id="audioError" error={error || status?.error || statusError} />
    {status?.enabled && status.pcm_bytes_received !== undefined && <details className="audio-diagnostics"><summary>{text('audioDiagnostics')}</summary>
      <p className="small-note">{text('audioByteCounts', { pcm: status.pcm_bytes_received, mp3: status.bytes_streamed || 0 })}</p>
      {status.receiver_diagnostics && <pre>{status.receiver_diagnostics}</pre>}
      {status.encoder_diagnostics && <pre>{status.encoder_diagnostics}</pre>}
    </details>}
    <audio id="browserAudio" ref={audio} preload="none" onPlaying={() => { if (joinedRef.current) setPhase('audioJoined'); }} onWaiting={() => { if (joinedRef.current) setPhase('audioBuffering'); }}
      onError={() => { if (joinedRef.current) { leave(); setError(text('audioFailed')); } }} onEnded={() => { if (joinedRef.current) { leave(); setError(text('audioEnded')); } }} />
  </section>;
}
