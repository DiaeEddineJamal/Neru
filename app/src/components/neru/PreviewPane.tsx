import { useCallback, useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent, type ReactNode, type RefObject } from 'react'
import { listen } from '@tauri-apps/api/event'
import { ArrowLeft, ArrowRight, Bug, Camera, Check, ChevronDown, CircleAlert, Copy, Download, EllipsisVertical, ExternalLink, Globe, Laptop, LoaderCircle, PenLine, Lock, MessageSquarePlus, Monitor, MousePointer2, Network, Play, Plus, RotateCw, Send, Server, Smartphone, Square, StickyNote, Tablet, Terminal, Trash2, TriangleAlert, X } from 'lucide-react'
import { api } from '../../api'
import {
  DEVICES, ZOOMS, describePicked, elementChipLabel, elementName, errorsMessage, normalizeUrl, notesMessage, pickedMessage, readPref, shortUrl, toRealUrl, writePref,
  type ConsoleEntry, type ConsoleLevel, type Device, type NetEntry, type Note, type Picked, type Route, type TabRequest,
} from '../../lib/browser'
import type { BrowserTabs } from '../../lib/useBrowserTabs'
import { MarkupBoard } from './Markup'
import { t as tx } from '../../i18n'

type Mode = 'off' | 'inspect' | 'annotate'
type Drawer = 'console' | 'network' | 'notes' | 'element' | null
type Zoom = 'fit' | number
type Shot = { dataUrl: string; width: number; height: number }
export type PreviewImage = { name: string; dataUrl: string }

/**
 * The page itself. Neru's window is cross-origin isolated (for on-device speech), which blocks
 * ordinary iframes of other origins ("refused to connect"). A credentialless iframe may load
 * them; the attribute has to be set before the frame navigates, and React drops it when it is
 * rendered as an empty string, so it is set by hand before `src`.
 */
function PreviewFrame({ src, frameRef, onLoad }: { src: string; frameRef: RefObject<HTMLIFrameElement | null>; onLoad: () => void }) {
  useEffect(() => {
    const node = frameRef.current
    if (!node) return
    node.setAttribute('credentialless', '')
    node.src = src
  }, [src, frameRef])
  return <iframe ref={frameRef} className="pv-frame" title={tx('App preview')} allow="clipboard-write; fullscreen" onLoad={() => { if (frameRef.current?.getAttribute('src')) onLoad() }} />
}

/** A small anchored menu that closes on outside click and Escape. */
function Menu({ label, icon, children, active, title, variant = 'tool', badge }: { label?: ReactNode; icon?: ReactNode; children: (close: () => void) => ReactNode; active?: boolean; title?: string; variant?: 'tool' | 'icon'; badge?: boolean }) {
  const [open, setOpen] = useState(false)
  const box = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!open) return
    const close = (event: MouseEvent | KeyboardEvent) => {
      if (event instanceof KeyboardEvent) { if (event.key === 'Escape') setOpen(false); return }
      if (!box.current?.contains(event.target as Node)) setOpen(false)
    }
    const away = () => setOpen(false)
    window.addEventListener('mousedown', close); window.addEventListener('keydown', close); window.addEventListener('blur', away)
    return () => { window.removeEventListener('mousedown', close); window.removeEventListener('keydown', close); window.removeEventListener('blur', away) }
  }, [open])
  return <div className="pv-menu" ref={box}>
    {variant === 'icon'
      ? <button type="button" className={`icon-button pv-icon${active || open ? ' on' : ''}`} title={title} aria-label={title} aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen(value => !value)}>{icon}{badge && <i className="pv-dot" />}</button>
      : <button type="button" className={`pv-tool${active || open ? ' on' : ''}`} title={title} aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen(value => !value)}>{icon}<span>{label}</span><ChevronDown size={12} /></button>}
    {open && <div className={`pv-menu-panel${variant === 'icon' ? ' right' : ''}`} role="menu">{children(() => setOpen(false))}</div>}
  </div>
}

const deviceIcon = (device: Device) => device.kind === 'phone' ? <Smartphone size={14} /> : device.kind === 'tablet' ? <Tablet size={14} /> : device.kind === 'desktop' ? <Laptop size={14} /> : <Monitor size={14} />
const clock = (t: number) => new Date(t).toLocaleTimeString([], { hour12: false })

/** One browser tab: address bar, page, and the inspection panels. Tabs stay mounted so pages keep running. */
export function BrowserTab({ id, projectKey, projectName, active, request, onMeta, onOpenExternal, onSendToAgent }: {
  id: number
  projectKey: string
  projectName: string
  /** This tab is the one on screen, in an open browser pane. */
  active: boolean
  request: TabRequest | null
  onMeta: (id: number, meta: { title: string; url: string; loading: boolean }) => void
  onOpenExternal: (url: string) => void
  onSendToAgent?: (text: string, images: PreviewImage[], element?: { label: string; detail: string }) => void
}) {
  const frameRef = useRef<HTMLIFrameElement>(null)
  const stageRef = useRef<HTMLDivElement>(null)
  const urlRef = useRef<HTMLInputElement>(null)
  const urlFocused = useRef(false)
  const routeRef = useRef<Route | null>(null)
  const handled = useRef(0)
  const entryId = useRef(0)
  const snapId = useRef(0)
  const snaps = useRef(new Map<number, { resolve: (shot: Shot) => void; reject: (error: Error) => void }>())
  const bridgeTimer = useRef(0)

  const [route, setRoute] = useState<Route | null>(null)
  const [nonce, setNonce] = useState(0)
  const [urlInput, setUrlInput] = useState('')
  const [realUrl, setRealUrl] = useState('')
  const [title, setTitle] = useState('')
  const [loading, setLoading] = useState(false)
  const [canBack, setCanBack] = useState(false)
  const [canForward, setCanForward] = useState(false)
  const [bridgeReady, setBridgeReady] = useState(false)
  const [bridgeMissing, setBridgeMissing] = useState(false)
  const [hint, setHint] = useState('')
  const [hintUrl, setHintUrl] = useState('')
  const [busy, setBusy] = useState(false)
  const [progress, setProgress] = useState('')
  const [error, setError] = useState('')

  const [deviceId, setDeviceId] = useState(() => readPref('neru.browser.device', 'fit'))
  const [landscape, setLandscape] = useState(() => readPref('neru.browser.landscape', false))
  const [custom, setCustom] = useState(() => readPref('neru.browser.custom', { w: 1024, h: 768 }))
  const [zoom, setZoom] = useState<Zoom>(() => readPref<Zoom>('neru.browser.zoom', 'fit'))
  const [stage, setStage] = useState({ w: 0, h: 0 })

  const [mode, setMode] = useState<Mode>('off')
  const [drawer, setDrawer] = useState<Drawer>(null)
  const [drawerHeight, setDrawerHeight] = useState(() => readPref('neru.browser.drawer', 260))
  const [logs, setLogs] = useState<ConsoleEntry[]>([])
  const [requests, setRequests] = useState<NetEntry[]>([])
  const [level, setLevel] = useState<'all' | 'error' | 'warn'>('all')
  const [filter, setFilter] = useState('')
  const [preserve, setPreserve] = useState(false)
  const [failedOnly, setFailedOnly] = useState(false)
  const [notes, setNotes] = useState<Note[]>([])
  const [picked, setPicked] = useState<Picked | null>(null)
  const [attachShot, setAttachShot] = useState(false)
  const [shot, setShot] = useState<Shot | null>(null)
  // A snapshot being drawn on (the markup board), like the Claude app's browser.
  const [markup, setMarkup] = useState<Shot | null>(null)
  const [shooting, setShooting] = useState(false)
  const [copied, setCopied] = useState('')

  const device = useMemo<Device>(() => deviceId === 'custom' ? { id: 'custom', name: 'Custom', w: custom.w, h: custom.h, kind: 'custom' } : DEVICES.find(item => item.id === deviceId) ?? DEVICES[0], [deviceId, custom])
  const fill = device.kind === 'fill'
  const page = useMemo(() => { try { return new URL(realUrl).pathname } catch { return '' } }, [realUrl])
  const pageRef = useRef(page)
  pageRef.current = page
  const pageNotes = useMemo(() => notes.filter(note => note.page === page), [notes, page])
  const tools = Boolean(route?.bridged && bridgeReady)
  const errorCount = logs.filter(item => item.level === 'error').length + requests.filter(item => !item.ok).length
  const secure = realUrl.startsWith('https://')

  // ---------- talking to the page bridge ----------
  const command = useCallback((type: string, data: Record<string, unknown> = {}) => {
    frameRef.current?.contentWindow?.postMessage({ __neruCmd: 1, type, ...data }, '*')
  }, [])

  const shoot = useCallback(() => new Promise<Shot>((resolve, reject) => {
    const id = ++snapId.current
    const timer = window.setTimeout(() => { snaps.current.delete(id); reject(new Error(tx('The page took too long to capture'))) }, 15000)
    snaps.current.set(id, { resolve: value => { window.clearTimeout(timer); resolve(value) }, reject: cause => { window.clearTimeout(timer); reject(cause) } })
    command('snapshot', { id })
  }), [command])

  useEffect(() => { if (bridgeReady) command('annotations', { notes: pageNotes.map(({ id, n, selector, region, comment, text }) => ({ id, n, selector, region, comment, text })) }) }, [pageNotes, bridgeReady, command])
  useEffect(() => { if (bridgeReady) command('mode', { mode }) }, [mode, bridgeReady, command])

  // ---------- navigation ----------
  const go = useCallback(async (raw: string) => {
    const url = normalizeUrl(raw)
    if (!url) { setError(tx('Type a localhost address (like localhost:3000) or an https address.')); return }
    setError(''); setLoading(true); setBridgeReady(false); setBridgeMissing(false); setMode('off')
    try {
      const next = await api.previewRoute(url)
      routeRef.current = next
      setRoute(next); setRealUrl(next.realUrl); setUrlInput(next.realUrl); setTitle(''); setCanBack(false); setCanForward(false)
      setNonce(value => value + 1)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause)); setLoading(false)
    }
  }, [])

  const reload = useCallback(() => {
    setLoading(true)
    if (bridgeReady) command('reload'); else setNonce(value => value + 1)
  }, [bridgeReady, command])

  const onFrameLoad = useCallback(() => {
    setLoading(false)
    window.clearTimeout(bridgeTimer.current)
    if (routeRef.current?.bridged) bridgeTimer.current = window.setTimeout(() => setBridgeMissing(true), 2500)
  }, [])

  useEffect(() => {
    void api.previewHint().then(value => { setHint(current => current || value.command); setHintUrl(value.url) }).catch(() => setHint(''))
    const stop = listen<string>('preview://status', event => setProgress(event.payload))
    return () => { void stop.then(fn => fn()); window.clearTimeout(bridgeTimer.current) }
  }, [projectKey])

  const start = useCallback(async () => {
    setBusy(true); setError(''); setProgress(tx('Getting the preview ready…'))
    try {
      const value = await api.previewStart()
      await go(value.url); setHint(value.command)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause))
    } finally { setBusy(false); setProgress('') }
  }, [go])

  useEffect(() => {
    if (!request || request.seq === handled.current) return
    handled.current = request.seq
    if (request.url) void go(request.url); else if (request.start) void start()
  }, [request, go, start])

  useEffect(() => { onMeta(id, { title, url: realUrl, loading }) }, [id, title, realUrl, loading, onMeta])

  const stopServer = async () => { await api.previewStop(); setRoute(null); routeRef.current = null; setBridgeReady(false); setLoading(false); setMode('off') }

  // ---------- messages from the page ----------
  const flash = (name: string) => { setCopied(name); window.setTimeout(() => setCopied(''), 1400) }
  const shortcut = useCallback((key: string) => {
    if (key === 'l') { urlRef.current?.focus(); urlRef.current?.select() }
    else if (key === 'r') reload()
    else window.dispatchEvent(new KeyboardEvent('keydown', { key, ctrlKey: true, bubbles: true }))
  }, [reload])

  useEffect(() => {
    const onMessage = (event: MessageEvent) => {
      const node = frameRef.current
      const data = event.data as { __neru?: number; type?: string; [key: string]: unknown } | null
      if (!node || event.source !== node.contentWindow || !data || data.__neru !== 1) return
      const current = routeRef.current
      switch (data.type) {
        case 'ready':
          window.clearTimeout(bridgeTimer.current)
          setBridgeReady(true); setBridgeMissing(false); setLoading(false)
          setLogs(items => preserve ? items : []); setRequests(items => preserve ? items : [])
          break
        case 'loading': setLoading(true); break
        case 'nav': {
          const href = toRealUrl(String(data.href ?? ''), current)
          setRealUrl(href); setTitle(String(data.title ?? '')); setCanBack(Boolean(data.back)); setCanForward(Boolean(data.forward)); setLoading(Boolean(data.loading))
          if (!urlFocused.current) setUrlInput(href)
          break
        }
        case 'console': case 'error': {
          const entry: ConsoleEntry = { id: ++entryId.current, level: data.type === 'error' ? 'error' : (String(data.level) as ConsoleLevel), kind: data.type === 'error' ? String(data.kind ?? '') : undefined, text: String(data.text ?? ''), stack: data.stack ? String(data.stack) : undefined, t: Number(data.t) || Date.now() }
          setLogs(items => [...items.slice(-499), entry])
          break
        }
        case 'net': {
          const entry: NetEntry = { id: ++entryId.current, method: String(data.method ?? 'GET'), url: toRealUrl(String(data.url ?? ''), current), status: Number(data.status) || 0, ms: Number(data.ms) || 0, ok: Boolean(data.ok), failure: String(data.failure ?? ''), t: Number(data.t) || Date.now() }
          setRequests(items => [...items.slice(-299), entry])
          break
        }
        case 'picked': setPicked(data.element as Picked); setDrawer('element'); setMode('off'); break
        case 'mode': setMode(data.mode === 'inspect' || data.mode === 'annotate' ? data.mode : 'off'); break
        case 'annotation:add': {
          const note = data.note as Omit<Note, 'n' | 'page'>
          setNotes(items => [...items, { ...note, n: items.length + 1, page: pageRef.current }])
          setDrawer('notes')
          break
        }
        case 'annotation:update': {
          const note = data.note as Pick<Note, 'id' | 'comment'>
          setNotes(items => items.map(item => item.id === note.id ? { ...item, comment: note.comment } : item))
          break
        }
        case 'annotation:delete': setNotes(items => items.filter(item => item.id !== data.id).map((item, index) => ({ ...item, n: index + 1 }))); break
        case 'snapshot': {
          const waiting = snaps.current.get(Number(data.id))
          snaps.current.delete(Number(data.id))
          if (!waiting) break
          if (data.error) waiting.reject(new Error(String(data.error)))
          else waiting.resolve({ dataUrl: String(data.dataUrl), width: Number(data.width), height: Number(data.height) })
          break
        }
        case 'key': shortcut(String(data.key)); break
        default: break
      }
    }
    window.addEventListener('message', onMessage)
    return () => window.removeEventListener('message', onMessage)
  }, [preserve, shortcut])

  // ---------- keyboard ----------
  useEffect(() => {
    if (!active) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && mode !== 'off') { setMode('off'); return }
      if ((event.ctrlKey || event.metaKey) && !event.altKey) {
        const key = event.key.toLowerCase()
        if (key === 'l') { event.preventDefault(); urlRef.current?.focus(); urlRef.current?.select() }
        else if (key === 'r' && route) { event.preventDefault(); reload() }
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [active, mode, route, reload])

  // ---------- layout ----------
  useEffect(() => {
    const node = stageRef.current
    if (!node) return
    const watch = new ResizeObserver(() => setStage({ w: node.clientWidth, h: node.clientHeight }))
    watch.observe(node)
    setStage({ w: node.clientWidth, h: node.clientHeight })
    return () => watch.disconnect()
  }, [])

  const size = fill ? { w: 0, h: 0 } : landscape ? { w: device.h, h: device.w } : { w: device.w, h: device.h }
  const margin = fill ? 0 : 28
  const fit = fill ? 1 : Math.max(0.1, Math.min(1, (stage.w - margin * 2) / size.w, (stage.h - margin * 2 - 22) / size.h))
  const scale = zoom === 'fit' ? fit : zoom
  const view = fill ? { w: Math.max(1, Math.floor(stage.w / scale)), h: Math.max(1, Math.floor(stage.h / scale)) } : size
  const percent = `${Math.round(scale * 100)}%`
  const viewportLabel = fill ? '' : `${device.name} ${size.w}×${size.h}`

  const choose = (id: string) => { setDeviceId(id); writePref('neru.browser.device', id) }
  const rotate = () => { setLandscape(value => { writePref('neru.browser.landscape', !value); return !value }) }
  const chooseZoom = (value: Zoom) => { setZoom(value); writePref('neru.browser.zoom', value) }
  const openDrawer = (next: Exclude<Drawer, null>) => setDrawer(value => value === next ? null : next)

  const toggleMode = (next: Exclude<Mode, 'off'>) => setMode(value => value === next ? 'off' : next)

  const dragDrawer = (event: ReactPointerEvent<HTMLDivElement>) => {
    const start = event.clientY, from = drawerHeight
    const move = (moved: PointerEvent) => setDrawerHeight(Math.max(140, Math.min(560, from + start - moved.clientY)))
    const end = () => { window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', end); setDrawerHeight(value => { writePref('neru.browser.drawer', value); return value }) }
    window.addEventListener('pointermove', move); window.addEventListener('pointerup', end)
  }

  // ---------- actions ----------
  const capture = async () => {
    setShooting(true); setError('')
    try { setShot(await shoot()) } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)) } finally { setShooting(false) }
  }

  const draw = async () => {
    setShooting(true); setError(''); setMode('off')
    try { setShot(null); setMarkup(await shoot()) } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)) } finally { setShooting(false) }
  }

  const sendNotes = async () => {
    if (!onSendToAgent || !pageNotes.length) return
    const images: PreviewImage[] = []
    if (attachShot) { try { const image = await shoot(); images.push({ name: 'preview-with-notes.jpg', dataUrl: image.dataUrl }) } catch { /* send the text on its own */ } }
    onSendToAgent(notesMessage(pageNotes, realUrl, viewportLabel), images)
  }

  const copy = async (name: string, text: string) => { try { await navigator.clipboard.writeText(text); flash(name) } catch { setError(tx('Copying was blocked')) } }

  const visibleLogs = logs.filter(item => (level === 'all' || (level === 'error' ? item.level === 'error' : item.level === 'warn' || item.level === 'error')) && (!filter || item.text.toLowerCase().includes(filter.toLowerCase())))
  const visibleRequests = requests.filter(item => (!failedOnly || !item.ok) && (!filter || item.url.toLowerCase().includes(filter.toLowerCase())))
  const problems = [...logs.filter(item => item.level === 'error'), ...requests.filter(item => !item.ok)]
  const logEnd = useRef<HTMLDivElement>(null)
  useEffect(() => { if (drawer === 'console') logEnd.current?.scrollIntoView({ block: 'end' }) }, [logs.length, drawer])

  const closeDrawer = <button type="button" className="icon-button pv-drawer-close" aria-label={tx('Close panel')} title={tx('Close panel')} onClick={() => setDrawer(null)}><X size={14} /></button>

  const disabledTip = !route ? tx('Open a page first') : !route.bridged ? tx('Page tools work on local pages. This is an external site.') : !bridgeReady ? tx('Waiting for the page…') : ''

  const port = (() => { try { return new URL(hintUrl).port } catch { return '' } })()

  return <div className="pv" data-mode={mode} hidden={!active}>
    <div className="pv-bar">
      <button className="icon-button" aria-label={tx('Back')} title={tx('Back')} disabled={!canBack || !tools} onClick={() => command('back')}><ArrowLeft size={16} /></button>
      <button className="icon-button" aria-label={tx('Forward')} title={tx('Forward')} disabled={!canForward || !tools} onClick={() => command('forward')}><ArrowRight size={16} /></button>
      {loading
        ? <button className="icon-button" aria-label={tx('Stop loading')} title={tx('Stop loading')} onClick={() => { command('stop'); setLoading(false) }}><X size={16} /></button>
        : <button className="icon-button" aria-label={tx('Reload')} title={`${tx('Reload')} (Ctrl+R)`} disabled={!route} onClick={reload}><RotateCw size={15} /></button>}
      <label className="pv-url" title={title || undefined}>
        {secure && <Lock size={12} />}
        <input ref={urlRef} value={urlInput} onChange={event => setUrlInput(event.target.value)} spellCheck={false} aria-label={tx('Address')} placeholder={tx('Type a URL')}
          onFocus={event => { urlFocused.current = true; event.currentTarget.select() }}
          onBlur={() => { urlFocused.current = false; if (realUrl) setUrlInput(realUrl) }}
          onKeyDown={event => { if (event.key === 'Enter') { void go(urlInput); event.currentTarget.blur() } else if (event.key === 'Escape') { setUrlInput(realUrl); event.currentTarget.blur() } }} />
      </label>
      <button type="button" className={`icon-button pv-icon${mode === 'inspect' ? ' on' : ''}`} disabled={!tools} title={disabledTip || tx('Select an element to inspect it or send it to the agent')} aria-label={tx('Select an element')} aria-pressed={mode === 'inspect'} onClick={() => toggleMode('inspect')}><MousePointer2 size={15} /></button>
      <button type="button" className={`icon-button pv-icon${mode === 'annotate' ? ' on' : ''}`} disabled={!tools} title={disabledTip || tx('Comment on an element or an area')} aria-label={tx('Annotate')} aria-pressed={mode === 'annotate'} onClick={() => toggleMode('annotate')}><MessageSquarePlus size={15} />{pageNotes.length > 0 && <b className="pv-badge">{pageNotes.length}</b>}</button>
      <button type="button" className={`icon-button pv-icon${markup ? ' on' : ''}`} disabled={!tools || shooting} title={disabledTip || tx('Draw on the page: pen, shapes, arrows and text')} aria-label={tx('Draw on the page')} aria-pressed={Boolean(markup)} onClick={() => markup ? setMarkup(null) : void draw()}><PenLine size={15} /></button>
      <button type="button" className="icon-button pv-icon" disabled={!tools || shooting} title={disabledTip || tx('Capture a screenshot of the page')} aria-label={tx('Screenshot')} onClick={() => void capture()}>{shooting ? <LoaderCircle size={15} className="animate-spin" /> : <Camera size={15} />}</button>
      <Menu variant="icon" icon={<EllipsisVertical size={16} />} title={tx('More')} badge={errorCount > 0}>
        {close => <>
          <div className="pv-menu-label">{tx('Device')}</div>
          {DEVICES.map(item => <button key={item.id} role="menuitemradio" aria-checked={deviceId === item.id} className={deviceId === item.id ? 'on' : ''} onClick={() => { choose(item.id); close() }}>{deviceIcon(item)}<span>{tx(item.name)}</span>{item.w > 0 && <em>{item.w}×{item.h}</em>}{deviceId === item.id && <Check size={13} />}</button>)}
          <div className="pv-custom"><span>{tx('Custom')}</span>
            <input type="number" min={200} max={4000} value={custom.w} aria-label={tx('Width')} onChange={event => { const next = { ...custom, w: Number(event.target.value) || 0 }; setCustom(next); writePref('neru.browser.custom', next) }} />
            <i>×</i>
            <input type="number" min={200} max={4000} value={custom.h} aria-label={tx('Height')} onChange={event => { const next = { ...custom, h: Number(event.target.value) || 0 }; setCustom(next); writePref('neru.browser.custom', next) }} />
            <button onClick={() => { if (custom.w >= 200 && custom.h >= 200) { choose('custom'); close() } }}>{tx('Use')}</button>
          </div>
          {!fill && <button role="menuitem" onClick={() => { rotate(); close() }}><RotateCw size={13} /><span>{tx('Rotate')}</span><em>{landscape ? tx('Landscape') : tx('Portrait')}</em></button>}
          <div className="pv-menu-label">{tx('Zoom')}</div>
          <div className="pv-zooms">
            <button role="menuitemradio" aria-checked={zoom === 'fit'} className={zoom === 'fit' ? 'on' : ''} onClick={() => { chooseZoom('fit'); close() }}>{tx('Fit')}</button>
            {ZOOMS.map(value => <button key={value} role="menuitemradio" aria-checked={zoom === value} className={zoom === value ? 'on' : ''} onClick={() => { chooseZoom(value); close() }}>{Math.round(value * 100)}%</button>)}
          </div>
          <div className="pv-menu-label">{tx('Panels')}</div>
          <button role="menuitem" disabled={!route} onClick={() => { openDrawer('console'); close() }}><Terminal size={13} /><span>{tx('Console')}</span>{errorCount > 0 && <b className="pv-count bad">{errorCount > 99 ? '99+' : errorCount}</b>}</button>
          <button role="menuitem" disabled={!route} onClick={() => { openDrawer('network'); close() }}><Network size={13} /><span>{tx('Network')}</span></button>
          <button role="menuitem" disabled={!route} onClick={() => { openDrawer('notes'); close() }}><StickyNote size={13} /><span>{tx('Annotations')}</span>{notes.length > 0 && <b className="pv-count">{notes.length}</b>}</button>
          <button role="menuitem" disabled={!picked} onClick={() => { openDrawer('element'); close() }}><Bug size={13} /><span>{tx('Selected element')}</span></button>
          <div className="pv-menu-sep" />
          <button role="menuitem" disabled={!(realUrl || urlInput)} onClick={() => { onOpenExternal(realUrl || normalizeUrl(urlInput)); close() }}><ExternalLink size={13} /><span>{tx('Open in your browser')}</span></button>
          <button role="menuitem" disabled={busy} onClick={() => { void start(); close() }}><Play size={13} /><span>{route ? tx('Restart the preview') : tx('Run the preview')}</span></button>
          <button role="menuitem" onClick={() => { void stopServer(); close() }}><Square size={13} /><span>{tx('Stop the preview server')}</span></button>
        </>}
      </Menu>
    </div>

    {loading && <div className="pv-loading" aria-hidden />}
    {progress && route && <p className="preview-hint">{progress}</p>}
    {error && <div className="pv-error"><CircleAlert size={14} /><pre className="preview-error">{error}</pre><button className="icon-button" aria-label={tx('Dismiss')} onClick={() => setError('')}><X size={14} /></button></div>}
    {route && route.bridged && bridgeMissing && !bridgeReady && <p className="pv-note"><TriangleAlert size={13} /> {tx('The page loaded without Neru’s page tools, so Inspect, Annotate and the console are unavailable here. It may not be an HTML page.')}</p>}
    {route && !route.bridged && <p className="pv-note"><Globe size={13} /> {tx('External site. Inspect, Annotate and the console work on local pages. Some sites refuse to load in a frame; use “Open in your browser” for those.')}</p>}

    <div className={`pv-stage${fill ? ' fill' : ' device'}${mode !== 'off' ? ' armed' : ''}`} ref={stageRef}>
      {route
        ? <div className="pv-device">
          {!fill && <div className="pv-caption">{tx(device.name)} · {size.w} × {size.h} · {percent}</div>}
          <div className="pv-viewport" style={{ width: Math.floor(view.w * scale), height: Math.floor(view.h * scale) }}>
            <div className="pv-scaler" style={{ width: view.w, height: view.h, transform: scale === 1 ? undefined : `scale(${scale})` }}>
              <PreviewFrame key={nonce} src={route.frameUrl} frameRef={frameRef} onLoad={onFrameLoad} />
            </div>
          </div>
        </div>
        : <div className="pv-empty-state">
          {busy
            ? <><LoaderCircle size={22} className="animate-spin" /><p>{progress || tx('Starting…')}</p></>
            : <>
              <button type="button" className="pv-server" disabled={busy} onClick={() => void start()} title={hint ? tx('Runs {command}', { command: hint }) : tx('Start the preview')}>
                <Server size={15} /><strong className="truncate">{projectName}</strong>{port && <em>:{port}</em>}<span className="pv-server-run"><Play size={13} /></span>
              </button>
              <p>{tx('Run a server to preview your app, or enter a URL.')}</p>
            </>}
        </div>}
      {mode !== 'off' && <div className="pv-mode-banner" role="status">{mode === 'inspect' ? tx('Click an element to inspect it') : tx('Click an element or drag an area to comment · Esc to stop')}<button onClick={() => setMode('off')}>{tx('Done')}</button></div>}
      {markup && <MarkupBoard shot={markup} onClose={() => setMarkup(null)} onSelect={() => { setMarkup(null); setMode('inspect') }}
        onSend={onSendToAgent ? dataUrl => onSendToAgent('', [{ name: 'preview-markup.jpg', dataUrl }]) : undefined} />}
      {shot && <div className="pv-shot">
        <img src={shot.dataUrl} alt={tx('Screenshot of the page')} />
        <div className="pv-shot-actions">
          <button onClick={() => { onSendToAgent?.('Screenshot of the running preview' + (realUrl ? ` (${realUrl})` : '') + '.\n\n', [{ name: 'preview.jpg', dataUrl: shot.dataUrl }]); setShot(null) }} disabled={!onSendToAgent}><Send size={13} /> {tx('Add to chat')}</button>
          <button onClick={() => { setMarkup(shot); setShot(null) }}><PenLine size={13} /> {tx('Mark up')}</button>
          <a href={shot.dataUrl} download={`preview-${Date.now()}.jpg`}><Download size={13} /> {tx('Save')}</a>
          <button className="icon-button" aria-label={tx('Close screenshot')} onClick={() => setShot(null)}><X size={14} /></button>
        </div>
      </div>}
    </div>

    {drawer && route && <div className="pv-drawer" style={{ height: drawerHeight }}>
      <div className="pv-drag" onPointerDown={dragDrawer} role="separator" aria-orientation="horizontal" aria-label={tx('Resize panel')} />
      {drawer === 'console' && <>
        <div className="pv-drawer-bar">
          <strong>{tx('Console')}</strong>
          <div className="pv-chips">{(['all', 'warn', 'error'] as const).map(item => <button key={item} className={level === item ? 'on' : ''} onClick={() => setLevel(item)}>{item === 'all' ? tx('All') : item === 'warn' ? tx('Warnings') : tx('Errors')}</button>)}</div>
          <input className="pv-filter" placeholder={tx('Filter')} value={filter} onChange={event => setFilter(event.target.value)} aria-label={tx('Filter console')} />
          <label className="pv-check"><input type="checkbox" checked={preserve} onChange={event => setPreserve(event.target.checked)} /> {tx('Keep on reload')}</label>
          <span className="pv-grow" />
          <button className="pv-link" disabled={!problems.length || !onSendToAgent} onClick={() => onSendToAgent?.(errorsMessage(problems, realUrl), [])}><Send size={12} /> {tx('Send errors to agent')}</button>
          <button className="pv-link" onClick={() => setLogs([])}><Trash2 size={12} /> {tx('Clear')}</button>
          {closeDrawer}
        </div>
        <div className="pv-lines">
          {visibleLogs.length === 0 && <p className="pv-empty">{tools ? tx('Nothing logged yet.') : tx('The console fills in for local pages.')}</p>}
          {visibleLogs.map(item => <div key={item.id} className={`pv-line ${item.level}`}>
            <time>{clock(item.t)}</time>
            <span className="pv-msg">{item.kind && <em>{item.kind}</em>}{item.text}{item.stack && <details><summary>{tx('Stack')}</summary><pre>{item.stack}</pre></details>}</span>
          </div>)}
          <div ref={logEnd} />
        </div>
      </>}
      {drawer === 'network' && <>
        <div className="pv-drawer-bar">
          <strong>{tx('Network')}</strong>
          <input className="pv-filter" placeholder={tx('Filter by URL')} value={filter} onChange={event => setFilter(event.target.value)} aria-label={tx('Filter requests')} />
          <label className="pv-check"><input type="checkbox" checked={failedOnly} onChange={event => setFailedOnly(event.target.checked)} /> {tx('Failed only')}</label>
          <span className="pv-grow" />
          <button className="pv-link" onClick={() => setRequests([])}><Trash2 size={12} /> {tx('Clear')}</button>
          {closeDrawer}
        </div>
        <div className="pv-lines">
          {visibleRequests.length === 0 && <p className="pv-empty">{tools ? tx('No fetch or XHR requests yet.') : tx('Requests appear for local pages.')}</p>}
          {visibleRequests.map(item => <div key={item.id} className={`pv-req${item.ok ? '' : ' bad'}`} title={item.url + (item.failure ? `\n${item.failure}` : '')}>
            <b>{item.status || 'ERR'}</b><span>{item.method}</span><code className="truncate">{shortUrl(item.url)}</code><em>{item.ms} ms</em>
          </div>)}
        </div>
      </>}
      {drawer === 'notes' && <>
        <div className="pv-drawer-bar">
          <strong>{tx('Annotations')}</strong><span className="pv-sub">{notes.length ? tx('{n} on this project', { n: notes.length }) : tx('Use Annotate, then click an element')}</span>
          <span className="pv-grow" />
          <label className="pv-check" title={tx('Sends a picture of the page with your numbered markers. Needs a model that can see images.')}><input type="checkbox" checked={attachShot} onChange={event => setAttachShot(event.target.checked)} /> {tx('Attach screenshot')}</label>
          <button className="pv-link" disabled={!notes.length} onClick={() => { setNotes([]) }}><Trash2 size={12} /> {tx('Clear all')}</button>
          <button className="button primary pv-send" disabled={!pageNotes.length || !onSendToAgent} onClick={() => void sendNotes()}><Send size={13} /> {pageNotes.length ? tx('Send {n} to agent', { n: pageNotes.length }) : tx('Send to agent')}</button>
          {closeDrawer}
        </div>
        <div className="pv-lines">
          {notes.length === 0 && <p className="pv-empty">{tx('No annotations yet. Turn on Annotate, click an element (or drag an area) and say what should change. Then send them all to the agent at once.')}</p>}
          {notes.map(note => <div key={note.id} className={`pv-note-row${note.page === page ? '' : ' other'}`}>
            <button className="pv-pin" title={tx('Show on the page')} disabled={note.page !== page || !tools} onClick={() => command('focus', { id: note.id })}>{note.n}</button>
            <div>
              <code className="truncate">{note.selector || note.text}</code>
              <textarea rows={2} value={note.comment} aria-label={tx('Comment {n}', { n: note.n })} onChange={event => setNotes(items => items.map(item => item.id === note.id ? { ...item, comment: event.target.value } : item))} />
              {note.page !== page && <span className="pv-sub">{note.page ? tx('On {page}', { page: note.page }) : tx('On another page')}</span>}
            </div>
            <button className="icon-button" aria-label={tx('Delete annotation')} onClick={() => setNotes(items => items.filter(item => item.id !== note.id).map((item, index) => ({ ...item, n: index + 1 })))}><Trash2 size={14} /></button>
          </div>)}
        </div>
      </>}
      {drawer === 'element' && picked && <>
        <div className="pv-drawer-bar">
          <strong>{tx('Element')}</strong><code className="pv-tag">{elementName(picked)}</code>
          <span className="pv-grow" />
          <button className="pv-link" onClick={() => void copy('selector', picked.selector)}>{copied === 'selector' ? <Check size={12} /> : <Copy size={12} />} {tx('Copy selector')}</button>
          <button className="button primary pv-send" disabled={!onSendToAgent} onClick={() => onSendToAgent?.('', [], { label: elementChipLabel(picked), detail: pickedMessage(picked, realUrl).trim() })}><Send size={13} /> {tx('Add to chat')}</button>
          {closeDrawer}
        </div>
        <div className="pv-lines pv-inspect">
          <dl>
            <dt>{tx('Selector')}</dt><dd><code>{picked.selector}</code></dd>
            {picked.components.length > 0 && <><dt>{tx('Component')}</dt><dd>{picked.components.join(' ← ')}</dd></>}
            <dt>{tx('Size')}</dt><dd>{tx('{w} × {h} px · at {x}, {y}', { w: picked.rect.w, h: picked.rect.h, x: picked.page.x, y: picked.page.y })}</dd>
            {picked.text && <><dt>{tx('Text')}</dt><dd>{picked.text}</dd></>}
            {Object.entries(picked.attrs).map(([key, value]) => <div key={key} className="pv-kv"><dt>{key}</dt><dd>{value}</dd></div>)}
          </dl>
          <div className="pv-styles">{Object.entries(picked.styles).map(([key, value]) => <div key={key} className="pv-kv"><dt>{key}</dt><dd>{/color/i.test(key) && <i style={{ background: value }} />}{value}</dd></div>)}</div>
          <pre className="pv-html">{describePicked(picked).split('\nHTML: ')[1] ?? ''}</pre>
        </div>
      </>}
    </div>}
  </div>
}

const hostOf = (url: string) => { try { return new URL(url).host } catch { return url } }

/** The browser's tab strip, shown in the dock header: “New tab +” until a page is open. */
export function BrowserTabStrip({ browser }: { browser: BrowserTabs }) {
  return <div className="bt-strip" role="tablist" aria-label={tx('Browser tabs')}>
    {browser.tabs.map(tab => {
      const label = tab.title || (tab.url ? hostOf(tab.url) : tx('New tab'))
      return <div key={tab.id} className={`bt-tab${tab.id === browser.activeId ? ' active' : ''}`} role="tab" aria-selected={tab.id === browser.activeId}>
        <button type="button" className="bt-label" title={tab.url || label} onClick={() => browser.select(tab.id)}>
          {tab.loading ? <LoaderCircle size={13} className="animate-spin" aria-hidden /> : tab.url ? <Globe size={13} aria-hidden /> : null}
          <span className="truncate">{label}</span>
        </button>
        {(browser.tabs.length > 1 || tab.url) && <button type="button" className="bt-close" aria-label={tx('Close {name}', { name: label })} onClick={() => browser.close(tab.id)}><X size={12} /></button>}
      </div>
    })}
    <button type="button" className="bt-add" aria-label={tx('New tab')} title={tx('New tab')} onClick={() => browser.add()}><Plus size={15} /></button>
  </div>
}

/** The in-app browser: every tab stays mounted so pages, consoles and notes survive switching. */
export function PreviewPane({ browser, projectKey, projectName, open, onOpenExternal, onSendToAgent }: {
  browser: BrowserTabs
  projectKey: string
  projectName: string
  /** The browser pane is showing. */
  open: boolean
  onOpenExternal: (url: string) => void
  onSendToAgent?: (text: string, images: PreviewImage[], element?: { label: string; detail: string }) => void
}) {
  const { tabs, activeId, meta, adoptRunning } = browser
  useEffect(() => { adoptRunning() }, [projectKey, adoptRunning])
  return <div className="preview-pane browser-pane">
    {tabs.map(tab => <BrowserTab key={tab.id} id={tab.id} projectKey={projectKey} projectName={projectName} active={open && tab.id === activeId} request={tab.request} onMeta={meta} onOpenExternal={onOpenExternal} onSendToAgent={onSendToAgent} />)}
  </div>
}
