import { useEffect, useState } from 'react'
import { isTauri } from '@tauri-apps/api/core'
import { LoaderCircle, Smartphone } from 'lucide-react'
import { api } from '../../api'
import type { RemoteStatus } from '../../types'
import { cn } from '@/lib/utils'
import { CommandBlock } from './CliSettings'

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

  return <section className="settings-section cli-settings remote-settings"><h2>Phone</h2>
    <p className="settings-lede">Open Code sessions or Team conversations on your phone, read replies, approve changes and send messages. Conversations are encrypted with a key shared by your paired devices.</p>
    <div className="settings-row">
      <div><strong><Smartphone size={15} className="cli-status-icon" /> Neru Remote {on && <span className="cli-badge">{phones === 1 ? '1 phone connected' : `${phones} phones connected`}</span>}</strong>
        <p>{on ? `Keep this computer awake and Neru open. Local connections use port ${status?.port}.` : 'Turn on Remote to pair your phone.'}</p></div>
      {working ? <LoaderCircle size={16} className="animate-spin" aria-label="Working" /> : <button type="button" className={cn('switch', on && 'on')} role="switch" aria-checked={on} aria-label="Neru Remote" disabled={!desktop} onClick={() => void run(() => api.remoteSetEnabled(!on))} />}
    </div>
    {on && status && <>
      <div className="settings-row"><div><strong>Connect over the internet</strong><p>Use your phone on another Wi-Fi network or mobile data. The desktop must stay awake with internet access.</p></div><button type="button" className={cn('switch', status.internet && 'on')} role="switch" aria-checked={status.internet} aria-label="Connect over the internet" disabled={working} onClick={() => void run(() => api.remoteSetInternet(!status.internet))} /></div>
      {status.internet && <>
        <p className="cli-note">{status.endpoint ? 'Internet access is ready. Scan the updated pairing code below.' : status.internetError || 'Preparing the internet connector. The first use downloads a verified Cloudflare connector.'}</p>
        <p className="cli-note">Automatic access uses a temporary Cloudflare tunnel. Its address changes after the desktop restarts, so scan again. For a permanent connection, configure your own stable tunnel below.</p>
        <details><summary className="cli-note">Use a permanent tunnel address</summary><p className="cli-note">Route your tunnel to http://localhost:{status.port}, enable WebSocket support, and enter its secure WebSocket address.</p><input aria-label="Permanent WebSocket endpoint" placeholder="wss://neru.example.com" value={endpoint} onChange={e => setEndpoint(e.target.value)} /><button type="button" className="button subtle" disabled={working} onClick={() => void run(() => api.remoteSetEndpoint(endpoint), 'Internet address updated. Scan the pairing code again.')}>Save address</button></details>
      </>}
      <h3 className="cli-heading">Pair a phone</h3>
      <p className="cli-note">Open the Neru app on your phone and scan this code, or paste the link below into it.</p>
      <div className="remote-qr" role="img" aria-label="Pairing QR code" dangerouslySetInnerHTML={{ __html: status.qrSvg }} />
      <CommandBlock label="Pairing link" command={status.pairing} />
      {status.hosts.length === 0 && <p className="cli-note">No network address found. Connect this computer to Wi-Fi or Ethernet, then turn Remote off and on again.</p>}
      <p className="cli-note">{status.internet ? 'Phone can’t connect? Keep this computer awake, check internet access on both devices, and scan the current code after the tunnel is ready.' : `Phone can’t connect? Check that both are on the same Wi-Fi and that your firewall allows Neru on private networks.${windows ? ' Windows Security → Firewall & network protection → Allow an app through firewall.' : ''}`}</p>
      <p className="cli-note">Anyone with this code can steer Neru on this computer, so keep it to yourself.</p>
      <div className="settings-row">
        <div><strong>Reset pairing</strong><p>Makes a new code. Paired phones disconnect and need to scan again.</p></div>
        <button type="button" className="button subtle" disabled={working} onClick={() => void run(() => api.remoteResetPairing(), 'Pairing reset. Scan the new code on your phone.')}>Reset pairing</button>
      </div>
    </>}
  </section>
}
