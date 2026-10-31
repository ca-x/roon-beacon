import { useEffect, useRef, useState } from 'react';
import { ArrowLeft, ChevronLeft, ChevronRight, Home, LibraryBig, Music2, Play, Radio, Search } from 'lucide-react';
import { Button, Notice, TextField, useText } from './ui';
import { errorMessage, request } from './api';
import type { BrowseItem, BrowseOptions, BrowseResult } from './types';

const pageSize = 40;

function ItemArtwork({ item }: { item: BrowseItem }) {
  const [failed, setFailed] = useState(false);
  return <span className="item-art">
    {item.image_key && !failed ? <img src={`/api/player/image/${encodeURIComponent(item.image_key)}`} alt="" width={48} height={48} loading="lazy" onError={() => setFailed(true)} /> : item.hint === 'action' ? <Play aria-hidden="true" /> : item.input_prompt ? <Search aria-hidden="true" /> : <Music2 aria-hidden="true" />}
  </span>;
}

export function LibraryPanel({ ready, zoneId, session }: { ready: boolean; zoneId: string; session: string }) {
  const text = useText();
  const [hierarchy, setHierarchy] = useState<'browse' | 'search'>('browse');
  const [result, setResult] = useState<BrowseResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [prompt, setPrompt] = useState<BrowseItem | null>(null);
  const [input, setInput] = useState('');
  const controller = useRef<AbortController>();
  const mounted = useRef(true);
  const busyRef = useRef(false);
  const titleRef = useRef<HTMLHeadingElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const initialized = useRef(false);

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; controller.current?.abort(); };
  }, []);

  const openPrompt = (item: BrowseItem) => {
    setPrompt(item);
    setInput(item.input_prompt?.value || '');
  };
  useEffect(() => { if (prompt) inputRef.current?.focus(); }, [prompt]);

  const browse = async (options: BrowseOptions = {}, userInitiated = false, nextHierarchy = hierarchy) => {
    if (!ready || busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setError('');
    setNotice('');
    const abort = new AbortController();
    controller.current = abort;
    try {
      const data = await request<BrowseResult>('/api/player/browse', {
        session, hierarchy: nextHierarchy, count: pageSize, ...options, ...(zoneId ? { zone_id: zoneId } : {})
      }, abort.signal);
      if (!mounted.current || abort.signal.aborted) return;
      setPrompt(null);
      setInput('');
      if (data.list) {
        setResult({ ...data, items: Array.isArray(data.items) ? data.items : [], offset: data.offset ?? data.list.display_offset ?? 0 });
        if (userInitiated) requestAnimationFrame(() => titleRef.current?.focus({ preventScroll: true }));
      }
      if (data.item?.input_prompt) openPrompt(data.item);
      if (data.is_error === true) setError(data.message || data.item?.title || text('invalidResponse'));
      else if (data.message || (data.action && data.action !== 'list')) setNotice(data.message || data.item?.title || text('actionDone'));
    } catch (failure) {
      if (mounted.current && !abort.signal.aborted) setError(errorMessage(failure));
    } finally {
      if (mounted.current && !abort.signal.aborted) { setBusy(false); busyRef.current = false; }
    }
  };

  useEffect(() => {
    if (ready && !initialized.current) {
      initialized.current = true;
      void browse({ pop_all: true });
    }
  }, [ready]);

  const count = result?.list?.count || 0;
  const offset = result?.offset || 0;
  const items = result?.items || [];
  const unavailable = !ready || busy;
  const switchHierarchy = (next: 'browse' | 'search') => {
    if (unavailable || next === hierarchy) return;
    setHierarchy(next);
    setResult(null);
    setPrompt(null);
    void browse({ pop_all: true }, true, next);
  };
  return <section className="library" aria-labelledby="libraryHeading">
    <div className="section-heading"><div><p className="eyebrow">{text('collection')}</p><h1 id="libraryHeading">{text('library')}</h1></div><LibraryBig className="section-icon" aria-hidden="true" /></div>
    <div className="browse-tabs" role="group" aria-label={text('librarySource')}>
      <Button id="browseTab" data-hierarchy="browse" aria-pressed={hierarchy === 'browse'} disabled={unavailable} onClick={() => switchHierarchy('browse')}><LibraryBig aria-hidden="true" />{text('browse')}</Button>
      <Button id="searchTab" data-hierarchy="search" aria-pressed={hierarchy === 'search'} disabled={unavailable} onClick={() => switchHierarchy('search')}><Search aria-hidden="true" />{text('search')}</Button>
    </div>
    <div id="onboarding" className="onboarding" hidden={ready}>
      <div className="empty-symbol"><Radio aria-hidden="true" /></div>
      <h2>{text('welcome')}</h2><p>{text('setupIntro')}</p>
      <ol><li>{text('setupCore')}</li><li>{text('setupExtension')}</li><li>{text('setupAudio')}</li></ol>
      <p className="small-note">{text('notRaat')}</p>
    </div>
    <div id="browseContent" hidden={!ready} aria-busy={busy}>
      <div className="browse-navigation">
        <Button id="browseBack" className="icon-button" aria-label={text('back')} disabled={unavailable || !result?.list?.level} onClick={() => browse({ pop_levels: 1 }, true)}><ArrowLeft aria-hidden="true" /></Button>
        <Button id="browseRoot" className="icon-button" aria-label={text('root')} disabled={unavailable} onClick={() => browse({ pop_all: true }, true)}><Home aria-hidden="true" /></Button>
        <h2 id="browseTitle" ref={titleRef} tabIndex={-1}>{result?.list?.title || text(hierarchy)}</h2>
      </div>
      <p id="browseStatus" className="small-note" role="status">{busy ? text('loading') : notice || (result ? text('items', { count }) : text('loadLibrary'))}</p>
      <Notice id="browseError" error={error} />
      <form id="promptForm" className="prompt-form" hidden={!prompt} onSubmit={(event) => { event.preventDefault(); if (prompt) void browse({ item_key: prompt.item_key, input }, true); }}>
        <TextField id="browseInput" ref={inputRef} variant="outline" label={prompt?.input_prompt?.prompt || prompt?.title || text('search')}
          type={prompt?.input_prompt?.is_password ? 'password' : 'search'} autoComplete="off" maxLength={2000} aria-describedby="browseError"
          value={input} onChange={(event) => setInput(event.target.value)} />
        <div className="inline-actions"><Button id="promptSubmit" className="primary" type="submit" disabled={unavailable}>{prompt?.input_prompt?.action || text('continue')}</Button><Button id="promptCancel" className="text-button" onClick={() => { setPrompt(null); setInput(''); titleRef.current?.focus(); }}>{text('cancel')}</Button></div>
      </form>
      <ul id="browseItems" className="browse-items">
        {items.map((item, index) => <li key={`${item.item_key || item.title}:${index}`}><Button className={`browse-row${['action', 'action_list'].includes(item.hint || '') ? ' action-row' : ''}`}
          disabled={unavailable || !item.item_key} onClick={() => item.input_prompt ? openPrompt(item) : browse({ item_key: item.item_key }, true)}>
          <ItemArtwork item={item} /><span className="item-copy"><span className="item-title">{item.title}</span>{item.subtitle && <span className="item-subtitle">{item.subtitle}</span>}</span><ChevronRight className="row-chevron" aria-hidden="true" />
        </Button></li>)}
      </ul>
      <p id="browseEmpty" className="empty-list" hidden={!result || busy || items.length > 0}>{text('emptyList')}</p>
      <div className="pagination"><span id="pageInfo" className="small-note">{count ? text('page', { first: offset + 1, last: Math.min(offset + items.length, count), count }) : ''}</span><div className="inline-actions">
        <Button id="pagePrevious" className="icon-button" aria-label={text('previousPage')} disabled={unavailable || offset <= 0} onClick={() => browse({ offset: Math.max(0, offset - pageSize) }, true)}><ChevronLeft aria-hidden="true" /></Button>
        <Button id="pageNext" className="icon-button" aria-label={text('nextPage')} disabled={unavailable || !items.length || offset + items.length >= count} onClick={() => browse({ offset: offset + items.length }, true)}><ChevronRight aria-hidden="true" /></Button>
      </div></div>
    </div>
  </section>;
}
