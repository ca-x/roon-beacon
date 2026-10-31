import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';

interface RangeProps {
  id: string;
  label: string;
  context: string;
  value: number;
  min: number;
  max: number;
  step: number | 'any';
  disabled: boolean;
  describedBy?: string;
  format: (value: number) => string;
  commit: (value: number, context: string) => Promise<void>;
  onError: (error: unknown) => void;
  children?: (value: number) => ReactNode;
}

const adjustmentKeys = new Set(['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End', 'PageUp', 'PageDown']);

export function RemoteRange(props: RangeProps) {
  const input = useRef<HTMLInputElement>(null);
  const latest = useRef(props);
  latest.current = props;
  const gesture = useRef<{ context: string; active: boolean } | null>(null);
  const queue = useRef<{ value: number; context: string } | null>(null);
  const sending = useRef(false);
  const pending = useRef<string | null>(null);
  const submit = useRef<() => Promise<void>>(async () => {});
  const lastSubmitted = useRef<{ context: string; value: number } | null>(null);
  const mounted = useRef(true);
  const [draft, setDraft] = useState<{ context: string; value: number } | null>(null);

  useLayoutEffect(() => {
    if (gesture.current) gesture.current.active = false;
    queue.current = null;
    pending.current = null;
    lastSubmitted.current = null;
    setDraft(null);
  }, [props.context]);

  useEffect(() => {
    mounted.current = true;
    const node = input.current!;
    const change = async () => {
      const current = latest.current;
      if (!node.isConnected || current.disabled || (gesture.current && gesture.current.context !== current.context)) return;
      const value = Math.max(current.min, Math.min(current.max, Number(node.value)));
      if (!Number.isFinite(value)) return;
      if (lastSubmitted.current?.context === current.context && lastSubmitted.current.value === value) return;
      lastSubmitted.current = { context: current.context, value };
      queue.current = { value, context: current.context };
      pending.current = current.context;
      setDraft({ context: current.context, value });
      if (sending.current) return;
      sending.current = true;
      try {
        while (queue.current) {
          const next = queue.current;
          queue.current = null;
          if (!node.isConnected || latest.current.disabled || latest.current.context !== next.context) continue;
          await latest.current.commit(next.value, next.context);
        }
      } catch (error) {
        queue.current = null;
        if (mounted.current) latest.current.onError(error);
      } finally {
        sending.current = false;
        pending.current = null;
        if (mounted.current && !gesture.current?.active) setDraft(null);
      }
    };
    submit.current = change;
    node.addEventListener('change', change);
    return () => {
      mounted.current = false;
      queue.current = null;
      node.removeEventListener('change', change);
    };
  }, []);

  const begin = () => { gesture.current = { context: props.context, active: true }; lastSubmitted.current = null; };
  const end = () => {
    if (gesture.current) gesture.current.active = false;
  };
  const value = draft?.context === props.context && (gesture.current?.active || pending.current === props.context) ? draft.value : props.value;
  return <>
    <input ref={input} id={props.id} type="range" aria-label={props.label} aria-describedby={props.describedBy}
      min={props.min} max={props.max} step={props.step} value={value} disabled={props.disabled} aria-valuetext={props.format(value)}
      onPointerDown={begin} onPointerUp={() => { if (gesture.current?.active) void submit.current(); end(); }} onPointerCancel={end} onLostPointerCapture={end} onBlur={end}
      onKeyDown={(event) => { if (adjustmentKeys.has(event.key) && !gesture.current?.active) begin(); }}
      onKeyUp={(event) => { if (adjustmentKeys.has(event.key)) end(); }}
      onChange={(event) => {
        if (gesture.current && gesture.current.context !== props.context) {
          event.currentTarget.value = String(props.value);
          return;
        }
        setDraft({ context: props.context, value: Number(event.currentTarget.value) });
      }} />
    {props.children?.(value)}
  </>;
}
