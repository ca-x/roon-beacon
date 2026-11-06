import { useRef, useState } from 'react';
import * as Dialog from '@radix-ui/react-dialog';
import { ArrowRightLeft, Check, Minus, Plus, Radio, Repeat, Repeat1, RotateCcw, RotateCw, Shuffle, Speaker, X } from 'lucide-react';
import { Button, Notice, useText } from './ui';
import { errorMessage, request } from './api';
import type { Output, Zone } from './types';

export function TransportTools({ coreId, zone, zones, outputs, ready, refresh }: {
  coreId: string | null; zone?: Zone; zones: Zone[]; outputs: Output[]; ready: boolean; refresh: () => Promise<unknown>;
}) {
  const text = useText();
  const [open, setOpen] = useState(false);
  const [selected, setSelected] = useState<string[]>([]);
  const [destination, setDestination] = useState('');
  const [pending, setPending] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const busy = useRef(false);
  const allOutputs = [...new Map([...zones.flatMap((item) => item.outputs || []), ...outputs].map((output) => [output.output_id, output])).values()];
  const settings = zone?.settings;
  const loop = settings?.loop || 'disabled';
  const loopLabel = loop === 'loop_one' ? 'loopOne' : loop === 'loop' ? 'loopAll' : 'loopOff';
  const enabled = ready && Boolean(coreId) && !pending;
  const send = async (action: Record<string, unknown>) => {
    if (!ready || !coreId || busy.current) return;
    busy.current = true;
    setPending(true); setError(''); setNotice('');
    try {
      await request('/api/player/transport', { ...action, core_id: coreId });
      setNotice(text('actionDone'));
      await refresh();
    } catch (failure) { setError(errorMessage(failure)); }
    finally { busy.current = false; setPending(false); }
  };
  const changeSettings = (change: Record<string, unknown>) => send({ action: 'settings', zone_id: zone?.zone_id, ...change });
  const compatible = (output: Output) => selected.every((id) => id === output.output_id || Boolean(output.can_group_with_output_ids?.includes(id) && allOutputs.find((other) => other.output_id === id)?.can_group_with_output_ids?.includes(output.output_id)));
  const canGroup = selected.length > 1 && selected.every((id) => {
    const output = allOutputs.find((candidate) => candidate.output_id === id);
    return output && compatible(output);
  });
  const sourceCommand = (output: Output, control_key: string, command: string) => send({ action: 'source', output_id: output.output_id, control_key, command });
  return <div className="transport-tools">
    <div className="playback-options" role="group" aria-label={text('playbackModes')}>
      <Button className="mode-button" aria-label={text('shuffle')} aria-pressed={settings?.shuffle || false} disabled={!enabled || !settings} onClick={() => changeSettings({ shuffle: !settings?.shuffle })}><Shuffle aria-hidden="true" /></Button>
      <Button className="mode-button" aria-label={text(loopLabel)} aria-pressed={loop !== 'disabled'} disabled={!enabled || !settings} onClick={() => changeSettings({ loop_mode: loop === 'disabled' ? 'loop' : loop === 'loop' ? 'loop_one' : 'disabled' })}>{loop === 'loop_one' ? <Repeat1 aria-hidden="true" /> : <Repeat aria-hidden="true" />}</Button>
      <Button className="mode-button radio-mode" aria-pressed={settings?.auto_radio || false} disabled={!enabled || !settings} onClick={() => changeSettings({ auto_radio: !settings?.auto_radio })}><Radio aria-hidden="true" /><span>{text('autoRadio')}</span></Button>
    </div>
    <div className="seek-shortcuts">
      <Button className="text-button" aria-label={text('rewind10')} disabled={!enabled || !zone?.is_seek_allowed} onClick={() => send({ action: 'seek_relative', zone_id: zone?.zone_id, seconds: -10 })}><RotateCcw aria-hidden="true" /><span>10</span></Button>
      {typeof zone?.queue_items_remaining === 'number' && <span className="small-note">{text('queueRemaining', { count: zone.queue_items_remaining })}</span>}
      <Button className="text-button" aria-label={text('forward10')} disabled={!enabled || !zone?.is_seek_allowed} onClick={() => send({ action: 'seek_relative', zone_id: zone?.zone_id, seconds: 10 })}><span>10</span><RotateCw aria-hidden="true" /></Button>
    </div>
    <Dialog.Root open={open} onOpenChange={(next) => { setOpen(next); setError(''); setNotice(''); if (next) { setSelected(zone?.outputs?.map((output) => output.output_id) || []); setDestination(''); } }}>
      <Dialog.Trigger asChild><Button className="output-manager-trigger secondary" disabled={!ready}><Speaker aria-hidden="true" /><span>{text('manageOutputs')}</span><ArrowRightLeft aria-hidden="true" /></Button></Dialog.Trigger>
      <Dialog.Portal><Dialog.Overlay className="dialog-overlay" /><Dialog.Content className="output-dialog" aria-describedby="outputDescription">
        <div className="output-dialog-heading"><div><Dialog.Title>{text('outputHeading')}</Dialog.Title><Dialog.Description id="outputDescription" className="small-note">{text('outputDescription')}</Dialog.Description></div><Dialog.Close asChild><Button className="icon-button" aria-label={text('close')}><X aria-hidden="true" /></Button></Dialog.Close></div>
        <div className="output-dialog-scroll">
          <fieldset className="output-picker" disabled={!enabled}><legend>{text('roonVolume')}</legend>
            {!allOutputs.length && <p className="small-note">{text('noOutputs')}</p>}
            {allOutputs.map((output) => <div className="output-device" key={output.output_id}>
              <label className="output-choice"><input type="checkbox" checked={selected.includes(output.output_id)} disabled={!selected.includes(output.output_id) && !compatible(output)} onChange={(event) => setSelected(event.target.checked ? [...selected, output.output_id] : selected.filter((id) => id !== output.output_id))} /><span className="output-check" aria-hidden="true"><Check /></span><Speaker aria-hidden="true" /><span>{output.display_name}<small>{zones.find((candidate) => candidate.outputs?.some((member) => member.output_id === output.output_id))?.display_name || output.zone_id || ''}</small></span></label>
              {output.volume && ['number', 'db', 'incremental'].includes(output.volume.type) && <div className="device-step-volume"><Button className="icon-button" aria-label={text('volumeDown', { name: output.display_name })} onClick={() => send({ action: 'relative_volume', output_id: output.output_id, steps: -1 })}><Minus aria-hidden="true" /></Button><span>{output.volume.type === 'incremental' ? '—' : `${output.volume.value}${output.volume.type === 'db' ? ' dB' : ''}`}</span><Button className="icon-button" aria-label={text('volumeUp', { name: output.display_name })} onClick={() => send({ action: 'relative_volume', output_id: output.output_id, steps: 1 })}><Plus aria-hidden="true" /></Button></div>}
              {output.source_controls?.map((source) => <div className="source-controls" key={source.control_key}><span className="small-note">{source.display_name}{source.status && ` · ${source.status}`}</span><div className="inline-actions"><Button className="text-button" onClick={() => sourceCommand(output, source.control_key, 'convenience_switch')}>{text('switchSource')}</Button>{source.supports_standby && <><Button className="text-button" onClick={() => sourceCommand(output, source.control_key, 'standby')}>{text('standby')}</Button><Button className="text-button" onClick={() => sourceCommand(output, source.control_key, 'toggle_standby')}>{text('toggleStandby')}</Button></>}</div></div>)}
            </div>)}
          </fieldset>
          <p className="small-note">{text('groupHint')}</p><div className="inline-actions output-routing-actions"><Button className="primary" disabled={!enabled || !canGroup} onClick={() => send({ action: 'group', output_ids: selected })}>{text('groupOutputs')}</Button><Button className="text-button" disabled={!enabled || (zone?.outputs?.length || 0) < 2} onClick={() => send({ action: 'ungroup', output_ids: zone?.outputs?.map((output) => output.output_id) })}>{text('ungroupOutputs')}</Button><Button className="text-button" disabled={!enabled} onClick={() => send({ action: 'refresh' })}>{text('refreshOutputs')}</Button></div>
          <section className="transfer-section" aria-labelledby="transferHeading"><h3 id="transferHeading">{text('transferHeading')}</h3><p className="small-note">{text('transferHint')}</p><label className="transfer-destination"><span>{text('destination')}</span><select value={destination} disabled={!enabled || !zone} onChange={(event) => setDestination(event.target.value)}><option value="">{text('chooseDestination')}</option>{zones.filter((candidate) => candidate.zone_id !== zone?.zone_id).map((candidate) => <option key={candidate.zone_id} value={candidate.zone_id}>{candidate.display_name}</option>)}</select></label><Button className="secondary" disabled={!enabled || !zone || !destination || !zones.some((candidate) => candidate.zone_id === destination && candidate.zone_id !== zone.zone_id)} onClick={() => send({ action: 'transfer', from_zone_id: zone?.zone_id, to_zone_id: destination })}><ArrowRightLeft aria-hidden="true" />{text('transferQueue')}</Button></section>
          <div className="pause-all-row"><p className="small-note">{text('pauseAllHint')}</p><Button className="text-button" disabled={!enabled || !zones.length} onClick={() => send({ action: 'pause_all' })}>{text('pauseAll')}</Button></div>
        </div>
        <div className="output-dialog-footer"><div aria-live="polite" role="status" className="small-note">{pending ? text('sending') : notice}</div><Notice id="outputError" error={error} /><Dialog.Close asChild><Button className="secondary">{text('close')}</Button></Dialog.Close></div>
      </Dialog.Content></Dialog.Portal>
    </Dialog.Root>
    {!open && <><Notice id="transportError" error={error} /><p className="sr-only" role="status">{pending ? text('sending') : notice}</p></>}
  </div>;
}
