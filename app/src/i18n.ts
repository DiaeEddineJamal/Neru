// Neru in English, French, Arabic and Moroccan Darija. Strings are written in English in the code and wrapped in
// t(); each part in ./i18n maps the English text to its translations. A missing translation shows the English,
// so nothing is ever blank. Placeholders are {name}: t('Hid {n} secrets', { n: 2 }). Arabic and Darija switch the
// window to right to left. Changing the language reloads the window.

export type Lang = 'en' | 'fr' | 'ar' | 'ary'
export type LanguageSetting = 'system' | Lang
/** One part of the translations: English text to French, Arabic and Darija. */
export type Part = { fr: Record<string, string>; ar: Record<string, string>; ary: Record<string, string> }

export const languages: { value: LanguageSetting; label: string }[] = [
  { value: 'system', label: 'System' },
  { value: 'en', label: 'English' },
  { value: 'fr', label: 'Français' },
  { value: 'ar', label: 'العربية' },
  { value: 'ary', label: 'الدارجة المغربية (Darija)' },
]

const dicts: Record<Exclude<Lang, 'en'>, Record<string, string>> = { fr: {}, ar: {}, ary: {} }
const modules = import.meta.glob<{ default: Part }>('./i18n/*.ts', { eager: true })
for (const part of Object.values(modules)) for (const lang of ['fr', 'ar', 'ary'] as const) Object.assign(dicts[lang], part.default[lang])

const KEY = 'neru.language'
export function deviceLang(): Lang {
  const locale = (navigator.language || 'en').toLowerCase()
  if (locale.startsWith('fr')) return 'fr'
  if (locale.startsWith('ar-ma') || locale.startsWith('ary')) return 'ary'
  if (locale.startsWith('ar')) return 'ar'
  return 'en'
}
export function languageSetting(): LanguageSetting {
  try { return (localStorage.getItem(KEY) as LanguageSetting | null) ?? 'system' } catch { return 'system' }
}
const resolve = (setting: LanguageSetting): Lang => (setting === 'system' ? deviceLang() : setting)
let lang: Lang = resolve(languageSetting())

export const currentLang = () => lang
export const isRTL = (l: Lang = lang) => l === 'ar' || l === 'ary'

/** Sets <html lang dir> for the current language; call once at startup. */
export function applyDocumentLanguage() {
  document.documentElement.lang = lang === 'ary' ? 'ar-MA' : lang
  document.documentElement.dir = isRTL() ? 'rtl' : 'ltr'
}

/** Saves the choice and reloads the window in it. */
export function setLanguage(setting: LanguageSetting) {
  try { localStorage.setItem(KEY, setting) } catch { /* storage unavailable */ }
  lang = resolve(setting)
  location.reload()
}

/** `text` in the current language, with {placeholders} filled from `vars`. */
export function t(text: string, vars?: Record<string, string | number>): string {
  const out = lang === 'en' ? text : (dicts[lang][text] ?? text)
  return vars ? out.replace(/\{(\w+)\}/g, (whole, key: string) => (key in vars ? String(vars[key]) : whole)) : out
}
