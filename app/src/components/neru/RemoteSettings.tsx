import { useEffect, useState } from 'react'
import { isTauri } from '@tauri-apps/api/core'
import { LoaderCircle, Smartphone } from '@/components/icons'
import { api } from '../../api'
import type { RemoteStatus } from '../../types'
import { cn } from '@/lib/utils'
import { CommandBlock } from './CliSettings'
import { t } from '../../i18n'

/** Settings → Phone: pair the Neru phone app to watch and steer sessions and Team from the couch. */
export function RemoteSettings({ onError, onNotice }: { onError: (message: string) => void; onNotice: (message: string) => void }) {
  const [status, setStatus] = useState<RemoteStatus | null>(null)
  const [working, setWorking] = useState(false)
  const [endpoint, setEndpoint] = useState('')
  const desktop = isTauri()
  const windows = /Win/i.test(navigator.platform)

  useEffect(() => {
    if (!desktop) return
    void api.remoteStatus().then(s => { setStatus(s); if (!s.endpoint.includes('.trycloudflare.com')) setEndpoint(s.endpoint) }).catch(cause => onError(String(cause)))
    const unlisten = api.onRemoteClients(clients => setStatus(current => current && { ...current, clients }))
    const tunnel = api.onRemoteTunnel(() => { void api.remoteStatus().then(setStatus).catch(cause => onError(String(cause))) })
    return () => { void unlisten.then(stop => stop()); void tunnel.then(stop => stop()) }
  }, [desktop, onError])

  const run = async (action: () => Promise<RemoteStatus>, notice?: string) => {
    setWorking(true)
    try { setStatus(await action()); if (notice) onNotice(notice) } catch (cause) { onError(String(cause)) } finally { setWorking(false) }
  }
  const on = Boolean(status?.enabled)
  const phones = status?.clients ?? 0

  return <section className="settings-section cli-settings remote-settings"><h2>{t('Phone')}</h2>
    <p className="settings-lede">{t('Open Code sessions or Team conversations on your phone, read replies, approve changes and send messages. Conversations are encrypted with a key shared by your paired devices.')}</p>
    <div className="settings-row">
      <div><strong><Smartphone size={15} className="cli-status-icon" /> Neru Remote {on && <span className="cli-badge">{phones === 1 ? t('1 phone connected') : t('{n} phones connected', { n: phones })}</span>}</strong>
        <p>{on ? t('Keep this computer awake and Neru open. Local connections use port {port}.', { port: status?.port ?? '' }) : t('Turn on Remote to pair your phone.')}</p></div>
      {working ? <LoaderCircle size={16} className="animate-spin" aria-label={t('Working')} /> : <button type="button" className={cn('switch', on && 'on')} role="switch" aria-checked={on} aria-label="Neru Remote" disabled={!desktop} onClick={() => void run(() => api.remoteSetEnabled(!on))} />}
    </div>
    {on && status && <>
      <div className="settings-row"><div><strong>{t('Connect over the internet')}</strong><p>{t('Use your phone on another Wi-Fi network or mobile data. The desktop must stay awake with internet access.')}</p></div><button type="button" className={cn('switch', status.internet && 'on')} role="switch" aria-checked={status.internet} aria-label={t('Connect over the internet')} disabled={working} onClick={() => void run(() => api.remoteSetInternet(!status.internet))} /></div>
      {status.internet && <>
        <p className="cli-note">{status.endpoint ? t('Internet access is ready. Scan the updated pairing code below.') : status.internetError || t('Preparing the internet connector. The first use downloads a verified Cloudflare connector.')}</p>
        <p className="cli-note">{t('Automatic access uses a temporary Cloudflare tunnel. Its address changes after the desktop restarts, so scan again. For a permanent connection, configure your own stable tunnel below.')}</p>
        <details><summary className="cli-note">{t('Use a permanent tunnel address')}</summary><p className="cli-note">{t('Route your tunnel to {url}, enable WebSocket support, and enter its secure WebSocket address.', { url: `http://localhost:${status.port}` })}</p><input aria-label={t('Permanent WebSocket endpoint')} placeholder="wss://neru.example.com" value={endpoint} onChange={e => setEndpoint(e.target.value)} /><button type="button" className="button subtle" disabled={working} onClick={() => void run(() => api.remoteSetEndpoint(endpoint), t('Internet address updated. Scan the pairing code again.'))}>{t('Save address')}</button></details>
      </>}
      <h3 className="cli-heading">{t('Pair a phone')}</h3>
      <p className="cli-note">{t('Open the Neru app on your phone and scan this code, or paste the link below into it.')}</p>
      <div className="remote-qr" role="img" aria-label={t('Pairing QR code')} dangerouslySetInnerHTML={{ __html: status.qrSvg }} />
      <CommandBlock label={t('Pairing link')} command={status.pairing} />
      {status.hosts.length === 0 && <p className="cli-note">{t('No network address found. Connect this computer to Wi-Fi or Ethernet, then turn Remote off and on again.')}</p>}
      <p className="cli-note">{status.internet ? t('Phone can’t connect? Keep this computer awake, check internet access on both devices, and scan the current code after the tunnel is ready.') : windows ? t('Phone can’t connect? Check that both are on the same Wi-Fi and that your firewall allows Neru on private networks. Windows Security → Firewall & network protection → Allow an app through firewall.') : t('Phone can’t connect? Check that both are on the same Wi-Fi and that your firewall allows Neru on private networks.')}</p>
      <p className="cli-note">{t('Anyone with this code can steer Neru on this computer, so keep it to yourself.')}</p>
      <div className="settings-row">
        <div><strong>{t('Reset pairing')}</strong><p>{t('Makes a new code. Paired phones disconnect and need to scan again.')}</p></div>
        <button type="button" className="button subtle" disabled={working} onClick={() => void run(() => api.remoteResetPairing(), t('Pairing reset. Scan the new code on your phone.'))}>{t('Reset pairing')}</button>
      </div>
    </>}
  </section>
}
