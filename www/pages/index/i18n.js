// i18next setup for the page; the texts are in www/locales/<lang>.json. The page is
// pre-rendered in English and switches in the browser after hydration.
import i18n from 'i18next'
import { initReactI18next } from 'react-i18next'
import en from '../../locales/en.json'
import fr from '../../locales/fr.json'
import de from '../../locales/de.json'
import es from '../../locales/es.json'
import it from '../../locales/it.json'
import zh from '../../locales/zh.json'

export const LANGS = [
  ['en', 'English'],
  ['fr', 'Français'],
  ['de', 'Deutsch'],
  ['es', 'Español'],
  ['it', 'Italiano'],
  ['zh', '中文'],
]

const resources = Object.fromEntries(
  Object.entries({ en, fr, de, es, it, zh }).map(([lang, translation]) => [lang, { translation }]),
)

i18n.use(initReactI18next).init({
  resources,
  lng: 'en',
  fallbackLng: 'en',
  // React already escapes.
  interpolation: { escapeValue: false },
})

export default i18n

/** Saved choice, else the browser language when we have it, else English. */
export function initialLang() {
  try {
    const saved = localStorage.getItem('lang')
    if (resources[saved]) return saved
  } catch {}
  const browser = (navigator.language || 'en').slice(0, 2).toLowerCase()
  return resources[browser] ? browser : 'en'
}

export function applyLang(lang) {
  i18n.changeLanguage(lang)
  document.documentElement.lang = lang
}

/** A choice made in the menu, remembered for the next visits. */
export function setLang(lang) {
  applyLang(lang)
  try {
    localStorage.setItem('lang', lang)
  } catch {}
}
