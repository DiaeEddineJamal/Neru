/* Neru page bridge. The preview proxy injects this into local pages shown in the in-app browser.
 * It talks to Neru only through postMessage with the embedding window: page events go up
 * ({ __neru: 1, type, ... }), commands come down ({ __neruCmd: 1, type, ... }).
 * It never reads cookies or storage and only runs when the page is a direct child frame of Neru. */
(() => {
  'use strict';
  if (window.__neruBridge || window.parent === window) return;
  try { const ancestors = location.ancestorOrigins; if (ancestors && ancestors.length > 1) return; } catch (_) { /* not supported */ }
  window.__neruBridge = true;

  const post = (type, data) => { try { parent.postMessage(Object.assign({ __neru: 1, type }, data), '*'); } catch (_) { /* parent gone */ } };
  const clip = (text, max) => { const limit = max || 1200; return text.length > limit ? text.slice(0, limit) + '…' : text; };
  const fmt = value => {
    if (typeof value === 'string') return value;
    if (value instanceof Error) return value.stack || String(value);
    if (value instanceof Element) return '<' + value.tagName.toLowerCase() + (value.id ? '#' + value.id : '') + '>';
    try {
      const seen = new WeakSet();
      const out = JSON.stringify(value, (key, item) => {
        if (typeof item === 'function') return '[function]';
        if (typeof item === 'bigint') return String(item);
        if (item && typeof item === 'object') { if (seen.has(item)) return '[circular]'; seen.add(item); }
        return item;
      });
      return out === undefined ? String(value) : out;
    } catch (_) { return String(value); }
  };

  // ---------- console, errors, network ----------
  let windowStart = Date.now(), windowCount = 0;
  const allowed = () => {
    const now = Date.now();
    if (now - windowStart > 1000) { windowStart = now; windowCount = 0; }
    return ++windowCount <= 120;
  };
  ['log', 'info', 'warn', 'error', 'debug'].forEach(level => {
    const original = console[level];
    console[level] = function () {
      try { if (allowed()) post('console', { level, text: clip(Array.prototype.map.call(arguments, fmt).join(' ')), t: Date.now() }); } catch (_) { /* ignore */ }
      return original.apply(this, arguments);
    };
  });
  addEventListener('error', event => {
    const target = event.target;
    if (target && target !== window && (target.src || target.href)) {
      post('error', { kind: 'resource', text: 'Failed to load <' + String(target.tagName || '').toLowerCase() + '> ' + (target.src || target.href), t: Date.now() });
    } else {
      const where = event.filename ? ' (' + event.filename.replace(location.origin, '') + ':' + event.lineno + ':' + event.colno + ')' : '';
      post('error', { kind: 'exception', text: clip((event.message || 'Error') + where), stack: clip(String((event.error && event.error.stack) || ''), 1600), t: Date.now() });
    }
  }, true);
  addEventListener('unhandledrejection', event => post('error', { kind: 'rejection', text: 'Unhandled rejection: ' + clip(fmt(event.reason)), t: Date.now() }));

  const absolute = url => { try { return new URL(url, location.href).href; } catch (_) { return String(url); } };
  const net = (method, url, status, started, ok, failure) => {
    const href = absolute(url);
    if (href.indexOf('/__neru/') >= 0 || !allowed()) return;
    post('net', { method: String(method || 'GET').toUpperCase(), url: href, status, ms: Math.round(performance.now() - started), ok, failure: failure || '', t: Date.now() });
  };
  const nativeFetch = window.fetch;
  if (nativeFetch) {
    window.fetch = function (input, init) {
      const started = performance.now();
      const url = typeof input === 'string' ? input : (input && input.url) || String(input);
      const method = (init && init.method) || (input && input.method) || 'GET';
      return nativeFetch.apply(this, arguments).then(
        response => { net(method, url, response.status, started, response.ok); return response; },
        error => { net(method, url, 0, started, false, String(error)); throw error; });
    };
  }
  const xhrOpen = XMLHttpRequest.prototype.open, xhrSend = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.open = function (method, url) { this.__neru = { method, url: String(url) }; return xhrOpen.apply(this, arguments); };
  XMLHttpRequest.prototype.send = function () {
    const started = performance.now(), info = this.__neru;
    if (info) this.addEventListener('loadend', () => net(info.method, info.url, this.status, started, this.status >= 200 && this.status < 400, this.status ? '' : 'network error'));
    return xhrSend.apply(this, arguments);
  };

  // ---------- navigation ----------
  const canBack = () => { try { return window.navigation ? navigation.canGoBack : history.length > 1; } catch (_) { return false; } };
  const canForward = () => { try { return window.navigation ? navigation.canGoForward : false; } catch (_) { return false; } };
  const favicon = () => { const link = document.querySelector('link[rel~="icon"]'); return link ? absolute(link.getAttribute('href') || '') : ''; };
  let navTimer = 0;
  const reportNav = () => {
    clearTimeout(navTimer);
    navTimer = setTimeout(() => post('nav', { href: location.href, title: document.title, back: canBack(), forward: canForward(), loading: document.readyState !== 'complete', favicon: favicon() }), 30);
  };
  ['pushState', 'replaceState'].forEach(name => {
    const original = history[name];
    history[name] = function () { const result = original.apply(this, arguments); reportNav(); return result; };
  });
  ['popstate', 'hashchange', 'pageshow', 'load', 'DOMContentLoaded'].forEach(name => addEventListener(name, reportNav));
  addEventListener('beforeunload', () => post('loading', {}));
  const watchTitle = () => {
    const title = document.querySelector('title');
    if (title) new MutationObserver(reportNav).observe(title, { childList: true, characterData: true, subtree: true });
  };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', watchTitle); else watchTitle();

  // ---------- describing elements ----------
  const generated = /^(css-|sc-|jsx-|svelte-|_|Mui|ng-|astro-)|[0-9]{4,}|^[a-z]+-[a-z0-9]{6,}$/;
  const safe = value => { try { return CSS.escape(value); } catch (_) { return value; } };
  const query = selector => { try { return selector ? document.querySelector(selector) : null; } catch (_) { return null; } };
  const unique = selector => { try { return document.querySelectorAll(selector).length === 1; } catch (_) { return false; } };

  function selectorOf(element) {
    if (!element || element.nodeType !== 1) return '';
    const parts = [];
    let current = element;
    while (current && current.nodeType === 1 && current !== document.documentElement && parts.length < 8) {
      let part = current.tagName.toLowerCase();
      if (current.id && /^[A-Za-z][\w-]*$/.test(current.id) && !/\d{4,}/.test(current.id)) { parts.unshift('#' + safe(current.id)); break; }
      const testId = current.getAttribute('data-testid') || current.getAttribute('data-test');
      if (testId) { parts.unshift(part + '[data-testid="' + testId.replace(/"/g, '') + '"]'); break; }
      const classes = Array.prototype.filter.call(current.classList, name => !generated.test(name)).slice(0, 2);
      if (classes.length) part += '.' + classes.map(safe).join('.');
      const parent = current.parentElement;
      if (parent) {
        const same = Array.prototype.filter.call(parent.children, item => item.tagName === current.tagName);
        if (same.length > 1) part += ':nth-of-type(' + (same.indexOf(current) + 1) + ')';
      }
      parts.unshift(part);
      current = parent;
      if (unique(parts.join(' > '))) break;
    }
    const built = parts.join(' > ');
    if (unique(built)) return built;
    // Fall back to a strict positional path.
    const path = [];
    for (let node = element; node && node.nodeType === 1 && node !== document.body; node = node.parentElement) {
      const siblings = node.parentElement ? Array.prototype.indexOf.call(node.parentElement.children, node) + 1 : 1;
      path.unshift(node.tagName.toLowerCase() + ':nth-child(' + siblings + ')');
    }
    return 'body > ' + path.join(' > ');
  }

  function componentNames(element) {
    const names = [];
    const add = name => { if (name && names.indexOf(name) < 0 && names.length < 4) names.push(name); };
    try {
      const key = Object.keys(element).find(item => item.indexOf('__reactFiber$') === 0);
      if (key) {
        for (let fiber = element[key]; fiber && names.length < 4; fiber = fiber.return) {
          const type = fiber.type;
          if (type && typeof type !== 'string') add(type.displayName || type.name || (type.render && (type.render.displayName || type.render.name)));
        }
      }
      const vue = element.__vueParentComponent;
      for (let component = vue; component && names.length < 4; component = component.parent) add(component.type && (component.type.name || component.type.__name));
    } catch (_) { /* ignore */ }
    return names;
  }

  const STYLE_KEYS = ['color', 'backgroundColor', 'fontFamily', 'fontSize', 'fontWeight', 'lineHeight', 'letterSpacing', 'textAlign', 'display', 'position', 'width', 'height', 'margin', 'padding', 'border', 'borderRadius', 'gap', 'flexDirection', 'justifyContent', 'alignItems', 'gridTemplateColumns', 'boxShadow', 'opacity', 'zIndex', 'overflow'];
  const ATTRS = ['href', 'src', 'alt', 'type', 'name', 'placeholder', 'aria-label', 'role', 'data-testid', 'title', 'for', 'value'];

  function describe(element) {
    const rect = element.getBoundingClientRect();
    const computed = getComputedStyle(element);
    const styles = {};
    STYLE_KEYS.forEach(key => {
      const value = computed[key];
      if (!value || value === 'normal' || value === 'none' || value === 'auto' || value === '0px' || value === 'rgba(0, 0, 0, 0)' || value === 'static' || value === 'visible') return;
      if ((key === 'border' && /^0px/.test(value)) || (key === 'margin' && value === '0px') || (key === 'padding' && value === '0px')) return;
      styles[key] = value;
    });
    const attrs = {};
    ATTRS.forEach(name => { const value = element.getAttribute(name); if (value) attrs[name] = clip(value, 160); });
    const text = (element.innerText || element.textContent || '').replace(/\s+/g, ' ').trim();
    return {
      tag: element.tagName.toLowerCase(),
      id: element.id || '',
      classes: Array.prototype.slice.call(element.classList, 0, 8),
      selector: selectorOf(element),
      text: clip(text, 200),
      attrs,
      styles,
      components: componentNames(element),
      rect: { x: Math.round(rect.left), y: Math.round(rect.top), w: Math.round(rect.width), h: Math.round(rect.height) },
      page: { x: Math.round(rect.left + scrollX), y: Math.round(rect.top + scrollY) },
      html: clip(element.outerHTML.replace(/\s+/g, ' '), 700),
    };
  }

  const label = element => {
    const rect = element.getBoundingClientRect();
    const classes = Array.prototype.filter.call(element.classList, name => !generated.test(name)).slice(0, 2);
    return element.tagName.toLowerCase() + (element.id ? '#' + element.id : '') + (classes.length ? '.' + classes.join('.') : '') + '  ' + Math.round(rect.width) + '×' + Math.round(rect.height);
  };

  // ---------- overlay (inspect + annotate) ----------
  const CSS_TEXT = [
    ':host{all:initial}',
    '*{box-sizing:border-box;font-family:Inter,"Segoe UI",system-ui,sans-serif}',
    '.hl{position:fixed;border:2px solid #86b391;background:rgba(100,128,106,.16);border-radius:3px;pointer-events:none;transition:left .05s,top .05s,width .05s,height .05s}',
    '.hl[hidden],.pop[hidden],.drag[hidden]{display:none}',
    '.tag{position:absolute;left:-2px;top:-26px;padding:4px 7px;border-radius:5px;background:#243228;color:#e9f1ea;font:600 11px/1 ui-monospace,Consolas,monospace;white-space:nowrap;box-shadow:0 2px 8px rgba(0,0,0,.35)}',
    '.tag.below{top:auto;bottom:-26px}',
    '.drag{position:fixed;border:2px dashed #86b391;background:rgba(134,179,145,.12);pointer-events:none}',
    '.region{position:fixed;border:2px dashed rgba(134,179,145,.85);border-radius:3px;pointer-events:none}',
    '.pin{position:fixed;width:24px;height:24px;margin:-12px 0 0 -12px;border-radius:50%;background:#486b51;color:#fff;font:700 12px/20px Inter,system-ui,sans-serif;text-align:center;border:2px solid #fff;box-shadow:0 2px 10px rgba(0,0,0,.45);cursor:pointer;pointer-events:auto;user-select:none;transition:transform .12s}',
    '.pin:hover,.pin.focus{transform:scale(1.18);background:#5c8666}',
    '.pin.pulse{animation:pulse .9s ease-out 2}',
    '@keyframes pulse{0%{box-shadow:0 0 0 0 rgba(134,179,145,.9)}100%{box-shadow:0 0 0 16px rgba(134,179,145,0)}}',
    '.pop{position:fixed;width:288px;padding:10px;border-radius:12px;border:1px solid #3d413a;background:#1d201c;color:#f2f0e9;box-shadow:0 18px 50px rgba(0,0,0,.5);pointer-events:auto;font-size:13px}',
    '.pop .what{margin:0 0 7px;color:#a9ada3;font:11.5px/1.3 ui-monospace,Consolas,monospace;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}',
    '.pop textarea{display:block;width:100%;height:84px;resize:none;padding:8px 9px;border:1px solid #3d413a;border-radius:8px;outline:0;background:#151714;color:#f2f0e9;font:13px/1.45 Inter,system-ui,sans-serif}',
    '.pop textarea:focus{border-color:#7fa186}',
    '.pop .row{display:flex;gap:6px;justify-content:flex-end;margin-top:8px}',
    '.pop button{height:28px;padding:0 11px;border:1px solid #3d413a;border-radius:7px;background:#262a24;color:#dcdcd4;font:600 12px Inter,system-ui,sans-serif;cursor:pointer}',
    '.pop button:hover{background:#30352e}',
    '.pop button.primary{border-color:#486b51;background:#486b51;color:#fff}',
    '.pop button.primary:hover{background:#54785d}',
    '.pop button.danger{margin-right:auto;color:#e0a19a}',
    '.pop .hint{margin:6px 0 0;color:#8b8f86;font-size:11px}',
  ].join('');

  let host = null, root = null, hl = null, tag = null, pins = null, pop = null, dragBox = null, cursorStyle = null;
  let mode = 'off';
  let notes = [];
  let hovered = null;
  let editing = null; // { id?, element, selector, region, label }
  let raf = 0;

  function ui() {
    if (host && host.isConnected) return;
    host = document.createElement('neru-overlay');
    host.setAttribute('style', 'all:initial;position:fixed;left:0;top:0;width:0;height:0;z-index:2147483647;pointer-events:none;display:block');
    root = host.attachShadow({ mode: 'open' });
    root.innerHTML = '<style>' + CSS_TEXT + '</style><div class="hl" hidden><span class="tag"></span></div><div class="drag" hidden></div><div class="layer"></div>' +
      '<div class="pop" hidden><p class="what"></p><textarea placeholder="What should change here?" spellcheck="true"></textarea><div class="row"><button class="danger" data-act="delete">Delete</button><button data-act="cancel">Cancel</button><button class="primary" data-act="save">Save</button></div><p class="hint">Ctrl+Enter to save · Esc to cancel</p></div>';
    hl = root.querySelector('.hl'); tag = root.querySelector('.tag'); dragBox = root.querySelector('.drag'); pins = root.querySelector('.layer'); pop = root.querySelector('.pop');
    pop.addEventListener('click', event => {
      const action = event.target && event.target.getAttribute && event.target.getAttribute('data-act');
      if (action === 'save') saveEdit(); else if (action === 'cancel') closeEdit(); else if (action === 'delete') deleteEdit();
    });
    pop.querySelector('textarea').addEventListener('keydown', event => {
      event.stopPropagation();
      if (event.key === 'Escape') { event.preventDefault(); closeEdit(); }
      else if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) { event.preventDefault(); saveEdit(); }
    });
    (document.documentElement || document).appendChild(host);
  }

  function setCursor(on) {
    if (on && !cursorStyle) {
      cursorStyle = document.createElement('style');
      cursorStyle.textContent = '*{cursor:crosshair!important}neru-overlay *{cursor:auto!important}';
      (document.head || document.documentElement).appendChild(cursorStyle);
    } else if (!on && cursorStyle) { cursorStyle.remove(); cursorStyle = null; }
  }

  function setMode(next) {
    mode = next;
    ui();
    hovered = null;
    hl.hidden = true; dragBox.hidden = true;
    setCursor(mode !== 'off');
    if (mode === 'off') closeEdit();
    frame();
  }

  function showBox(element) {
    if (!element) { hl.hidden = true; return; }
    const rect = element.getBoundingClientRect();
    hl.hidden = false;
    hl.style.left = rect.left + 'px'; hl.style.top = rect.top + 'px'; hl.style.width = rect.width + 'px'; hl.style.height = rect.height + 'px';
    tag.textContent = label(element);
    tag.classList.toggle('below', rect.top < 30);
  }

  const inOverlay = event => { try { return event.composedPath().indexOf(host) >= 0; } catch (_) { return false; } };
  const elementAt = (x, y) => { const item = document.elementFromPoint(x, y); return item && item !== host && item.tagName !== 'NERU-OVERLAY' ? item : null; };

  // ---------- pointer handling while a mode is on ----------
  let down = null;
  // A pick ends the mode on pointer-up, but the click that follows must still not reach the page.
  let quietUntil = 0;
  const swallow = event => {
    if ((mode === 'off' && Date.now() > quietUntil) || inOverlay(event)) return;
    event.preventDefault(); event.stopImmediatePropagation();
  };
  ['mousedown', 'mouseup', 'click', 'dblclick', 'contextmenu', 'auxclick', 'touchstart', 'touchend', 'submit', 'dragstart', 'selectstart'].forEach(name => addEventListener(name, swallow, true));

  addEventListener('pointerdown', event => {
    if (mode === 'off' || inOverlay(event) || event.button !== 0) return;
    swallow(event);
    if (editing) closeEdit();
    down = { x: event.clientX, y: event.clientY, moved: false };
  }, true);
  addEventListener('pointermove', event => {
    if (mode === 'off' || inOverlay(event)) return;
    if (down && mode === 'annotate' && (down.moved || Math.abs(event.clientX - down.x) + Math.abs(event.clientY - down.y) > 8)) {
      down.moved = true;
      hl.hidden = true;
      dragBox.hidden = false;
      dragBox.style.left = Math.min(down.x, event.clientX) + 'px'; dragBox.style.top = Math.min(down.y, event.clientY) + 'px';
      dragBox.style.width = Math.abs(event.clientX - down.x) + 'px'; dragBox.style.height = Math.abs(event.clientY - down.y) + 'px';
      return;
    }
    hovered = elementAt(event.clientX, event.clientY);
    showBox(hovered);
  }, true);
  addEventListener('pointerup', event => {
    if (mode === 'off' || inOverlay(event) || !down) return;
    swallow(event);
    const start = down; down = null;
    if (mode === 'annotate' && start.moved) {
      dragBox.hidden = true;
      const x = Math.min(start.x, event.clientX), y = Math.min(start.y, event.clientY);
      const w = Math.abs(event.clientX - start.x), h = Math.abs(event.clientY - start.y);
      const target = elementAt(x + w / 2, y + h / 2);
      openEdit({ element: target, region: { x: Math.round(x + scrollX), y: Math.round(y + scrollY), w: Math.round(w), h: Math.round(h) }, text: 'Selected area ' + Math.round(w) + '×' + Math.round(h) });
      return;
    }
    const target = elementAt(event.clientX, event.clientY);
    if (!target) return;
    if (mode === 'inspect') {
      quietUntil = Date.now() + 400;
      post('picked', { element: describe(target) });
      post('mode', { mode: 'off' });
      setMode('off');
    } else {
      openEdit({ element: target, region: null, text: label(target) });
    }
  }, true);

  addEventListener('keydown', event => {
    if (event.key === 'Escape' && mode !== 'off' && !editing) { post('mode', { mode: 'off' }); setMode('off'); return; }
    if ((event.ctrlKey || event.metaKey) && !event.altKey && ['l', 'r', 'k', 'b', 'n', ',', 'f'].indexOf(event.key.toLowerCase()) >= 0) {
      if (['l', 'r', 'f'].indexOf(event.key.toLowerCase()) >= 0) event.preventDefault();
      post('key', { key: event.key.toLowerCase(), shift: event.shiftKey });
    }
  }, true);

  // ---------- annotation popover + pins ----------
  function openEdit(target) {
    ui();
    const element = target.element;
    editing = { id: target.id || '', element, region: target.region || null, selector: target.selector || (element ? selectorOf(element) : ''), text: target.text || '', comment: target.comment || '' };
    hl.hidden = true;
    pop.hidden = false;
    pop.querySelector('.what').textContent = editing.text || editing.selector;
    pop.querySelector('[data-act="delete"]').style.display = editing.id ? '' : 'none';
    const area = pop.querySelector('textarea');
    area.value = editing.comment;
    positionPop();
    setTimeout(() => area.focus(), 0);
  }
  function positionPop() {
    if (!editing || pop.hidden) return;
    let rect = null;
    if (editing.region) rect = { left: editing.region.x - scrollX, top: editing.region.y - scrollY, bottom: editing.region.y - scrollY + editing.region.h, right: editing.region.x - scrollX + editing.region.w };
    else { const element = editing.element || query(editing.selector); if (element) rect = element.getBoundingClientRect(); }
    if (!rect) rect = { left: innerWidth / 2 - 140, top: innerHeight / 3, bottom: innerHeight / 3 + 20, right: innerWidth / 2 + 140 };
    const height = pop.offsetHeight || 190;
    let left = Math.max(8, Math.min(innerWidth - 296, rect.left));
    let top = rect.bottom + 10;
    if (top + height > innerHeight - 8) top = Math.max(8, rect.top - height - 10);
    if (top + height > innerHeight - 8) top = Math.max(8, innerHeight - height - 8);
    pop.style.left = left + 'px'; pop.style.top = top + 'px';
  }
  function closeEdit() {
    editing = null;
    if (pop) pop.hidden = true;
    if (mode !== 'off') { /* stay armed for the next annotation */ }
  }
  function saveEdit() {
    if (!editing) return;
    const comment = pop.querySelector('textarea').value.trim();
    if (!comment) { pop.querySelector('textarea').focus(); return; }
    const id = editing.id || ('n' + Date.now().toString(36) + Math.random().toString(36).slice(2, 6));
    const note = { id, selector: editing.selector, region: editing.region, comment, text: editing.text };
    if (editing.id) post('annotation:update', { note });
    else post('annotation:add', { note: Object.assign({ element: editing.element ? describe(editing.element) : null }, note) });
    closeEdit();
  }
  function deleteEdit() {
    if (editing && editing.id) post('annotation:delete', { id: editing.id });
    closeEdit();
  }

  let focusId = '', focusUntil = 0;
  function renderPins() {
    if (!pins) return;
    const wanted = new Set();
    notes.forEach(note => {
      wanted.add(note.id);
      let pin = pins.querySelector('[data-id="' + note.id + '"]');
      if (!pin) {
        pin = document.createElement('div');
        pin.className = 'pin'; pin.setAttribute('data-id', note.id);
        pin.addEventListener('click', event => { event.stopPropagation(); const current = notes.find(item => item.id === note.id); if (current) openEdit({ id: current.id, selector: current.selector, region: current.region, element: query(current.selector), text: current.text, comment: current.comment }); });
        pins.appendChild(pin);
      }
      pin.textContent = String(note.n);
      const element = query(note.selector);
      let x, y, visible = true;
      if (element) { const rect = element.getBoundingClientRect(); x = rect.left + 4; y = rect.top + 4; visible = rect.width > 0 && rect.height > 0; }
      else if (note.region) { x = note.region.x - scrollX; y = note.region.y - scrollY; }
      else visible = false;
      if (visible) { x = Math.max(12, Math.min(innerWidth - 12, x)); y = Math.max(12, Math.min(innerHeight - 12, y)); }
      pin.style.display = visible ? '' : 'none';
      pin.style.left = x + 'px'; pin.style.top = y + 'px';
      pin.classList.toggle('pulse', note.id === focusId && Date.now() < focusUntil);
      let region = pins.querySelector('[data-region="' + note.id + '"]');
      if (note.region) {
        if (!region) { region = document.createElement('div'); region.className = 'region'; region.setAttribute('data-region', note.id); pins.appendChild(region); }
        region.style.left = (note.region.x - scrollX) + 'px'; region.style.top = (note.region.y - scrollY) + 'px';
        region.style.width = note.region.w + 'px'; region.style.height = note.region.h + 'px';
      } else if (region) region.remove();
    });
    Array.prototype.slice.call(pins.children).forEach(child => {
      const id = child.getAttribute('data-id') || child.getAttribute('data-region');
      if (!wanted.has(id)) child.remove();
    });
  }
  function frame() {
    cancelAnimationFrame(raf);
    if (!host) return;
    renderPins();
    if (editing) positionPop();
    if (mode !== 'off' && hovered && !down) showBox(hovered);
    if (notes.length || editing || mode !== 'off') raf = requestAnimationFrame(frame);
  }

  // ---------- snapshot ----------
  async function snapshot() {
    const width = innerWidth, height = innerHeight;
    const dataUrl = async url => {
      try {
        const response = await fetch(url, { credentials: 'omit' });
        if (!response.ok) return null;
        const blob = await response.blob();
        if (blob.size > 700000) return null;
        return await new Promise(resolve => { const reader = new FileReader(); reader.onload = () => resolve(reader.result); reader.onerror = () => resolve(null); reader.readAsDataURL(blob); });
      } catch (_) { return null; }
    };
    let css = '';
    Array.prototype.forEach.call(document.styleSheets, sheet => { try { css += Array.prototype.map.call(sheet.cssRules, rule => rule.cssText).join('\n') + '\n'; } catch (_) { /* cross-origin sheet */ } });
    const assets = Array.from(new Set(Array.from(css.matchAll(/url\((['"]?)(?!data:|#)([^'")]+)\1\)/g)).map(match => match[2]))).slice(0, 36);
    await Promise.all(assets.map(async url => { const data = await dataUrl(absolute(url)); if (data) css = css.split(url).join(data); }));

    const clone = document.documentElement.cloneNode(true);
    clone.querySelectorAll('script,noscript,neru-overlay,link[rel~="stylesheet"],style,iframe,video,audio').forEach(node => node.remove());
    const originals = Array.prototype.slice.call(document.querySelectorAll('img'));
    const copies = Array.prototype.slice.call(clone.querySelectorAll('img'));
    await Promise.all(copies.map(async (copy, index) => {
      const source = originals[index];
      copy.removeAttribute('srcset'); copy.removeAttribute('loading');
      const data = source && source.currentSrc && index < 40 ? await dataUrl(source.currentSrc) : null;
      if (data) copy.setAttribute('src', data);
    }));
    const canvases = Array.prototype.slice.call(document.querySelectorAll('canvas'));
    Array.prototype.forEach.call(clone.querySelectorAll('canvas'), (copy, index) => {
      try { const image = document.createElement('img'); image.setAttribute('src', canvases[index].toDataURL()); image.setAttribute('style', copy.getAttribute('style') || ''); image.setAttribute('width', copy.width); image.setAttribute('height', copy.height); copy.replaceWith(image); } catch (_) { /* tainted */ }
    });
    const fields = Array.prototype.slice.call(document.querySelectorAll('input,textarea,select'));
    Array.prototype.forEach.call(clone.querySelectorAll('input,textarea,select'), (copy, index) => {
      const source = fields[index];
      if (!source) return;
      if (copy.tagName === 'TEXTAREA') copy.textContent = source.value; else if (copy.tagName === 'INPUT') { copy.setAttribute('value', source.value); if (source.checked) copy.setAttribute('checked', ''); }
    });
    const sheet = document.createElement('style');
    sheet.textContent = css;
    const head = clone.querySelector('head') || clone.insertBefore(document.createElement('head'), clone.firstChild);
    head.appendChild(sheet);
    clone.setAttribute('xmlns', 'http://www.w3.org/1999/xhtml');
    clone.setAttribute('style', 'width:' + width + 'px;height:' + height + 'px;overflow:hidden;' + (clone.getAttribute('style') || ''));
    const body = clone.querySelector('body');
    if (body) body.style.cssText += ';position:relative;left:' + (-scrollX) + 'px;top:' + (-scrollY) + 'px;margin:0;';
    const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="' + width + '" height="' + height + '"><foreignObject x="0" y="0" width="100%" height="100%">' + new XMLSerializer().serializeToString(clone) + '</foreignObject></svg>';
    const image = new Image();
    await new Promise((resolve, reject) => { image.onload = resolve; image.onerror = () => reject(new Error('The page could not be rendered to an image')); image.src = 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg); });
    const scale = Math.min(2, Math.max(1, devicePixelRatio || 1), 1800 / Math.max(width, height));
    const canvas = document.createElement('canvas');
    canvas.width = Math.round(width * scale); canvas.height = Math.round(height * scale);
    const context = canvas.getContext('2d');
    context.fillStyle = getComputedStyle(document.body || document.documentElement).backgroundColor;
    if (!context.fillStyle || context.fillStyle === 'rgba(0, 0, 0, 0)') context.fillStyle = '#ffffff';
    context.fillRect(0, 0, canvas.width, canvas.height);
    context.drawImage(image, 0, 0, canvas.width, canvas.height);
    notes.forEach(note => {
      const element = query(note.selector);
      let x, y;
      if (element) { const rect = element.getBoundingClientRect(); x = rect.left + 4; y = rect.top + 4; } else if (note.region) { x = note.region.x - scrollX; y = note.region.y - scrollY; } else return;
      if (x < 0 || y < 0 || x > width || y > height) return;
      context.beginPath(); context.arc(x * scale, y * scale, 12 * scale, 0, Math.PI * 2); context.fillStyle = '#486b51'; context.fill();
      context.lineWidth = 2 * scale; context.strokeStyle = '#ffffff'; context.stroke();
      context.fillStyle = '#ffffff'; context.font = '700 ' + 12 * scale + 'px sans-serif'; context.textAlign = 'center'; context.textBaseline = 'middle'; context.fillText(String(note.n), x * scale, y * scale + scale);
    });
    return { dataUrl: canvas.toDataURL('image/jpeg', 0.86), width: canvas.width, height: canvas.height };
  }

  // ---------- commands from Neru ----------
  addEventListener('message', event => {
    if (event.source !== parent) return;
    const message = event.data;
    if (!message || message.__neruCmd !== 1) return;
    switch (message.type) {
      case 'mode': setMode(message.mode === 'inspect' || message.mode === 'annotate' ? message.mode : 'off'); break;
      case 'annotations': ui(); notes = Array.isArray(message.notes) ? message.notes : []; frame(); break;
      case 'focus': {
        const note = notes.find(item => item.id === message.id);
        const element = note && query(note.selector);
        if (element) element.scrollIntoView({ block: 'center', behavior: 'smooth' });
        else if (note && note.region) scrollTo({ top: Math.max(0, note.region.y - 120), behavior: 'smooth' });
        focusId = message.id; focusUntil = Date.now() + 2200; frame();
        break;
      }
      case 'reload': location.reload(); break;
      case 'stop': window.stop(); break;
      case 'back': history.back(); break;
      case 'forward': history.forward(); break;
      case 'scroll': scrollTo({ top: message.top || 0, behavior: 'smooth' }); break;
      case 'snapshot':
        snapshot().then(result => post('snapshot', Object.assign({ id: message.id }, result)), error => post('snapshot', { id: message.id, error: String((error && error.message) || error) }));
        break;
      default: break;
    }
  });

  post('ready', { href: location.href, title: document.title, width: innerWidth, height: innerHeight });
  reportNav();
})();
