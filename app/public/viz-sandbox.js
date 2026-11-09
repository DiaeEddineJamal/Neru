// Runs one ```html-widget from the conversation. Loaded by viz-sandbox.html inside
// <iframe sandbox="allow-scripts">; talks to the app only through postMessage.
(() => {
  'use strict'
  const host = window.parent
  if (host === window) return
  const send = message => host.postMessage({ source: 'neru-viz', ...message }, '*')
  let rendered = false

  const reportHeight = () => {
    const height = Math.ceil(document.documentElement.getBoundingClientRect().height)
    send({ type: 'height', height })
  }

  window.addEventListener('error', event => send({ type: 'error', message: String(event.message || 'Script error').slice(0, 300) }))
  window.addEventListener('unhandledrejection', event => send({ type: 'error', message: String(event.reason && event.reason.message || event.reason || 'Unhandled rejection').slice(0, 300) }))

  // Inline script is blocked by CSP, so each script becomes a blob: URL, run in document order.
  const runScript = (code, module) => new Promise(resolve => {
    const url = URL.createObjectURL(new Blob([code], { type: 'text/javascript' }))
    const script = document.createElement('script')
    if (module) script.type = 'module'
    script.src = url
    script.onload = script.onerror = () => { URL.revokeObjectURL(url); resolve() }
    document.body.appendChild(script)
  })

  // on*="..." attributes are inline script too: rewire them as listeners from one blob script.
  const handlerCode = root => {
    let next = 0
    let code = ''
    for (const element of root.querySelectorAll('*')) {
      for (const attribute of [...element.attributes]) {
        if (!/^on[a-z]+$/i.test(attribute.name)) continue
        const id = String(next++)
        element.setAttribute('data-neru-h' + id, '')
        element.removeAttribute(attribute.name)
        code += `(function(){var el=document.querySelector('[data-neru-h${id}]');if(el)el.addEventListener(${JSON.stringify(attribute.name.slice(2).toLowerCase())},function(event){${attribute.value}\n});})();\n`
      }
    }
    return code
  }

  const render = (html, theme) => {
    rendered = true
    document.documentElement.dataset.theme = theme === 'light' ? 'light' : 'dark'
    const parsed = new DOMParser().parseFromString(html, 'text/html')
    // javascript: links cannot run here anyway; drop them so they do not look clickable.
    for (const link of parsed.querySelectorAll('[href^="javascript:" i]')) link.removeAttribute('href')
    const scripts = []
    for (const script of parsed.querySelectorAll('script')) {
      if (!script.src) scripts.push({ code: script.textContent || '', module: script.type === 'module' })
      script.remove()
    }
    const handlers = handlerCode(parsed)
    for (const node of [...parsed.head.childNodes]) {
      if (node.nodeName === 'STYLE') document.head.appendChild(document.importNode(node, true))
    }
    for (const attribute of [...parsed.body.attributes]) if (!/^on/i.test(attribute.name)) document.body.setAttribute(attribute.name, attribute.value)
    document.body.replaceChildren(...[...parsed.body.childNodes].map(node => document.importNode(node, true)))
    new ResizeObserver(reportHeight).observe(document.body)
    reportHeight()
    void (async () => {
      for (const script of scripts) await runScript(script.code, script.module)
      if (handlers) await runScript(handlers, false)
      // Widgets often wait for these; they fired before the widget arrived.
      document.dispatchEvent(new Event('DOMContentLoaded', { bubbles: true }))
      window.dispatchEvent(new Event('load'))
      reportHeight()
    })()
  }

  window.addEventListener('message', event => {
    if (event.source !== host || rendered) return
    const data = event.data
    if (!data || data.source !== 'neru-viz-host' || data.type !== 'render' || typeof data.html !== 'string') return
    try { render(data.html, data.theme) } catch (error) { send({ type: 'error', message: String(error && error.stack || error).slice(0, 300) }) }
  })
  send({ type: 'ready' })
})()
