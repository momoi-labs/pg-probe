import en, { type Dictionary } from './en';
import es from './es';
import ptBR from './pt-BR';

// The site is built once per language, at its own path. English is the
// default, at the root. The order is the order of the language menu.
export const languages = ['en', 'pt-BR', 'es'] as const;
export type Language = (typeof languages)[number];

export const paths: Record<Language, string> = { en: '/', 'pt-BR': '/pt-br/', es: '/es/' };

// Each language named in itself, as the menu shows it.
export const names: Record<Language, string> = { en: 'English', 'pt-BR': 'Português', es: 'Español' };

const dictionaries: Record<Language, Dictionary> = { en, 'pt-BR': ptBR, es };

export const dictionary = (language: Language) => dictionaries[language];

// Where a visitor's choice is kept, as in pg-yard.
export const storageKey = 'pg-probe.language';
