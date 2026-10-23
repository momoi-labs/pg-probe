import { useMatches } from '@tanstack/react-router'
import { explain as explainEn, type Explain } from './explain.ts'
import esES from './locales/es-ES.ts'
import ptBR from './locales/pt-BR.ts'

// The printed report can be read in these languages. The console screens stay
// in English, the language of the source text.
export const locales = ['en-US', 'pt-BR', 'es-ES'] as const
export type Locale = (typeof locales)[number]

export const localeNames: Record<Locale, string> = {
  'en-US': 'English',
  'pt-BR': 'Português',
  'es-ES': 'Español',
}

export const isLocale = (value: unknown): value is Locale =>
  locales.includes(value as Locale)

// A translation: the English text of each message, as written in the source,
// to its text in the language, and the report explanations by panel id.
export type Translation = { messages: Record<string, string>; explain: Record<string, Explain> }

const translations: Record<Exclude<Locale, 'en-US'>, Translation> = {
  'pt-BR': ptBR,
  'es-ES': esES,
}

// English text with no translation, for the check in development.
const missing = new Set<string>()
if (import.meta.env.DEV) Object.assign(window, { i18nMissing: missing })

// The language of the report, from its ?lang= search parameter. Everywhere
// else it is English.
export const useLocale = (): Locale =>
  useMatches({
    select: (matches) => {
      const lang = matches.find((match) => match.routeId === '/$snapshotId/report')?.search
      return lang && 'lang' in lang && isLocale(lang.lang) ? lang.lang : 'en-US'
    },
  })

export type Translate = (text: string, values?: Record<string, string | number>) => string

// Translates English source text. Placeholders such as {count} take their
// value after the lookup, so one message covers every number.
export function useT(): Translate {
  const locale = useLocale()
  const messages = locale === 'en-US' ? undefined : translations[locale].messages
  return (text, values) => {
    let result = text
    if (messages) {
      const found = messages[text]
      if (found == null && import.meta.env.DEV) missing.add(text)
      result = found ?? text
    }
    if (values) {
      for (const [name, value] of Object.entries(values)) {
        // A function keeps $ patterns in the value as plain text.
        result = result.replaceAll(`{${name}}`, () => String(value))
      }
    }
    return result
  }
}

// The report explanations in the reader's language. One without a
// translation stays in English.
export function useExplain(): Record<string, Explain> {
  const locale = useLocale()
  if (locale === 'en-US') return explainEn
  const translated = translations[locale].explain
  if (import.meta.env.DEV) {
    for (const id of Object.keys(explainEn)) if (!translated[id]) missing.add(`explain: ${id}`)
  }
  return { ...explainEn, ...translated }
}
