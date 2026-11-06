import { useRef, useState } from 'react';
import { Music2, SkipBack, SkipForward, Play, Pause, Square, Volume2, VolumeX } from 'lucide-react';
import { RemoteRange } from './RemoteRange';
import { TransportTools } from './TransportTools';
import { Button, Notice, useText } from './ui';
import { clockTime, errorMessage, request } from './api';
import type { Output, Zone } from './types';

export function seekContext(coreId: string | null, zone?: Zone) {
  const track = zone?.now_playing;
  return JSON.stringify([coreId, zone?.zone_id, track?.length, track?.image_key, track?.one_line, track?.two_line, track?.three_line]);
}

function volumeContext(coreId: string | null, zone: Zone | undefined, output: Output) {
  const volume = output.volume;
  return JSON.stringify([coreId, zone?.zone_id, output.output_id, volume?.type, volume?.min, volume?.max, volume?.step]);
}

function OutputVolume({ coreId, zone, output, ready, refresh, onError, isCurrent }: {
  coreId: string | null; zone?: Zone; output: Output; ready: boolean; refresh: () => Promise<unknown>; onError: (error: unknown) => void;
  isCurrent: (context: string) => boolean;
}) {
  const text = useText();
  const [muting, setMuting] = useState(false);
  const volume = output.volume;
  const adjustable = Boolean(volume && ['number', 'db'].includes(volume.type) && Number.isFinite(volume.min) && Number.isFinite(volume.max) && volume.max > volume.min);
  const context = volumeContext(coreId, zone, output);
  const muteSupported = typeof volume?.is_muted === 'boolean';
  const id = `output-${output.output_id}`;
  const format = (amount: number) => volume?.type === 'db' ? `${amount} dB` : `${amount}`;
  return <div className="output-volume">
    <label htmlFor={id}>{output.display_name}</label>
    <div className="volume-line">
      <Button className="icon-button" disabled={!ready || !muteSupported || muting}
        aria-label={text(volume?.is_muted ? 'unmute' : 'mute', { name: output.display_name })}
        aria-pressed={muteSupported ? volume?.is_muted : undefined} onClick={async () => {
          setMuting(true);
          try { await request('/api/player/mute', { core_id: coreId, output_id: output.output_id, is_muted: !volume?.is_muted }); await refresh(); }
          catch (error) { onError(error); } finally { setMuting(false); }
        }}>{volume?.is_muted ? <VolumeX aria-hidden="true" /> : <Volume2 aria-hidden="true" />}</Button>
      <RemoteRange id={id} label={text('outputVolume', { name: output.display_name })} context={context}
        min={adjustable ? volume!.min : 0} max={adjustable ? volume!.max : 1} step={adjustable && (volume?.step || 0) > 0 ? volume!.step! : 'any'}
        value={adjustable ? Math.max(volume!.min, Math.min(volume!.max, volume!.value)) : 0} disabled={!ready || !adjustable}
        format={format} onError={onError} commit={async (value, current) => {
          if (!ready || current !== context || !isCurrent(current)) return;
          await request('/api/player/volume', { core_id: coreId, output_id: output.output_id, value });
          await refresh();
        }}>{(value) => <output htmlFor={id}>{adjustable ? format(value) : '—'}</output>}</RemoteRange>
    </div>
    <p className="small-note" hidden={adjustable && !volume?.is_muted}>{text(!adjustable ? volume?.type === 'fixed' ? 'fixedVolume' : 'unsupportedVolume' : 'muted')}</p>
  </div>;
}

export function PlayerPanel({ coreId, zone, zones, outputs, ready, refresh }: { coreId: string | null; zone?: Zone; zones: Zone[]; outputs: Output[]; ready: boolean; refresh: () => Promise<unknown> }) {
  const text = useText();
  const live = useRef({ coreId, zone, ready });
  live.current = { coreId, zone, ready };
  const [error, setError] = useState('');
  const [pending, setPending] = useState(false);
  const [failedImage, setFailedImage] = useState('');
  const [loadedImage, setLoadedImage] = useState('');
  const track = zone?.now_playing;
  const lines = track?.three_line || track?.two_line || track?.one_line;
  const title = lines?.line1 || text('nothingPlaying');
  const image = track?.image_key ? `/api/player/image/${encodeURIComponent(track.image_key)}` : '';
  const imageIdentity = `${coreId}:${image}`;
  const visibleImage = image && loadedImage === imageIdentity && failedImage !== imageIdentity;
  const length = Math.max(0, Number(track?.length) || 0);
  const position = Math.min(length, Math.max(0, Number(zone?.seek_position ?? track?.seek_position) || 0));
  const playing = zone?.state === 'playing';
  const controlReady = ready && Boolean(zone) && !pending;
  const onError = (failure: unknown) => setError(errorMessage(failure));
  const control = async (command: string) => {
    if (!zone || !controlReady) return;
    setPending(true);
    setError('');
    try { await request('/api/player/control', { core_id: coreId, zone_id: zone.zone_id, control: command }); await refresh(); }
    catch (failure) { onError(failure); } finally { setPending(false); }
  };
  const context = seekContext(coreId, zone);
  return <section className="player" aria-labelledby="nowPlayingHeading">
    <div className="player-top"><h2 id="nowPlayingHeading" className="eyebrow">{text('nowPlaying')}</h2><span id="playState" className="play-state">{text(zone?.state || 'stopped')}</span></div>
    <div className="artwork">
      {!visibleImage && <Music2 id="artworkPlaceholder" aria-hidden="true" />}
      <img id="coverArt" src={image || undefined} alt={text('cover', { title })} hidden={!visibleImage}
        onLoad={() => setLoadedImage(imageIdentity)} onError={() => setFailedImage(imageIdentity)} />
    </div>
    <div className="track-info"><h3 id="trackTitle">{title}</h3><p id="trackArtist">{lines?.line2 || text(zone ? 'pickMusic' : 'noZoneHint')}</p><p id="trackAlbum" className="small-note">{lines?.line3 || ''}</p></div>
    <div className="seek-control">
      <RemoteRange id="seek" label={text('seek')} describedBy="seekHint" context={context} min={0} max={Math.max(1, length)} step={1}
        value={position} disabled={!ready || !zone?.is_seek_allowed || length <= 0} format={(value) => `${clockTime(value)} / ${clockTime(length)}`}
        onError={onError} commit={async (seconds, current) => {
          const currentState = live.current;
          if (!currentState.ready || !currentState.zone?.is_seek_allowed || current !== seekContext(currentState.coreId, currentState.zone)) return;
          setError('');
          await request('/api/player/seek', { core_id: currentState.coreId, zone_id: currentState.zone.zone_id, seconds });
          await refresh();
        }}>{(value) => <div className="time-labels"><span id="elapsed">{clockTime(value)}</span><span id="duration">{clockTime(length)}</span></div>}</RemoteRange>
      <p id="seekHint" className="sr-only">{text('seekHint')}</p>
    </div>
    <div className="transport">
      <Button id="previous" className="icon-button" aria-label={text('previous')} disabled={!controlReady || !zone?.is_previous_allowed} onClick={() => control('previous')}><SkipBack aria-hidden="true" /></Button>
      <Button id="playPause" className="play-button" aria-label={text(playing ? 'pause' : 'play')} disabled={!controlReady || !(playing ? zone?.is_pause_allowed : zone?.is_play_allowed)} onClick={() => control(playing ? 'pause' : 'play')}>{playing ? <Pause aria-hidden="true" /> : <Play aria-hidden="true" />}</Button>
      <Button id="next" className="icon-button" aria-label={text('next')} disabled={!controlReady || !zone?.is_next_allowed} onClick={() => control('next')}><SkipForward aria-hidden="true" /></Button>
      <Button id="stop" className="icon-button stop-button" aria-label={text('stop')} disabled={!controlReady || !['playing', 'paused', 'loading'].includes(zone?.state || '')} onClick={() => control('stop')}><Square aria-hidden="true" /></Button>
    </div>
    <TransportTools key={`${coreId}:${zone?.zone_id}`} coreId={coreId} zone={zone} zones={zones} outputs={outputs} ready={ready} refresh={refresh} />
    <div className="output-volumes"><h3>{text('roonVolume')}</h3><p className="small-note" hidden={Boolean(zone?.outputs?.length)}>{text('noVolume')}</p><div id="volumeControls">
      {zone?.outputs?.map((output) => <OutputVolume key={output.output_id} coreId={coreId} zone={zone} output={output} ready={ready} refresh={refresh} onError={onError} isCurrent={(current) => {
        const currentState = live.current;
        const currentOutput = currentState.zone?.outputs?.find((candidate) => candidate.output_id === output.output_id);
        return Boolean(currentState.ready && currentOutput && current === volumeContext(currentState.coreId, currentState.zone, currentOutput));
      }} />)}
    </div></div>
    <Notice id="playerError" error={error} />
  </section>;
}
