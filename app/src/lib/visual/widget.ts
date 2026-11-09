/** Limits and location for ```html-widget fences. */

/** Widgets bigger than this are shown as code, not run. */
export const WIDGET_MAX_BYTES = 200 * 1024
export const WIDGET_MIN_HEIGHT = 40
export const WIDGET_MAX_HEIGHT = 1600
/** The sandbox page (app/public/viz-sandbox.html): runs a widget with no network and no app access. */
export const WIDGET_SANDBOX_URL = `${import.meta.env?.BASE_URL ?? '/'}viz-sandbox.html`
