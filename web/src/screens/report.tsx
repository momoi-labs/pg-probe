import { useIsFetching } from '@tanstack/react-query'
import { useNavigate, useSearch } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import { utcMinute } from '../format.tsx'
import { useLocale, useT } from '../i18n.ts'
import { useRange } from '../range.tsx'
import { Metrics } from './metrics.tsx'
import { NotCollected } from './not-collected.tsx'
import { Section } from './panels.tsx'
import { Queries } from './queries.tsx'

// Opens the print dialog once every table has loaded and the charts have
// drawn, then drops the flag so a reload does not print again.
function usePrintWhenReady(print: boolean | undefined) {
  const fetching = useIsFetching()
  const navigate = useNavigate()
  useEffect(() => {
    if (!print || fetching > 0) return
    const timer = setTimeout(() => {
      window.print()
      navigate({ to: '.', search: (prev) => ({ ...prev, print: undefined }), replace: true })
    }, 500)
    return () => clearTimeout(timer)
  }, [print, fetching, navigate])
}

type ContentsGroup = { title: string; entries: { id: string; title: string }[] }

// What a report pane draws when it has data: a chart, a table, a bar gauge or
// a meter. app.css uses the same list for page breaks and notes.
const drawn = 'figure, table, .bar-gauge, .meter'

// The charts and tables of the report by section, read from the drawn page
// once everything has loaded, and again in a new language, for the contents.
function useContents() {
  const fetching = useIsFetching()
  const locale = useLocale()
  const [sections, setSections] = useState<ContentsGroup[]>([])
  useEffect(() => {
    if (fetching > 0) return
    const timer = setTimeout(() => {
      const found: ContentsGroup[] = []
      for (const pane of document.querySelectorAll<HTMLElement>(`.report .grid-pane:has(${drawn})`)) {
        const id = pane.querySelector('.panel-anchor')?.id
        const title = pane.querySelector('.grid-pane-title')?.textContent
        const section = pane.closest('details')?.querySelector(':scope > summary')?.textContent ?? ''
        if (!id || !title) continue
        if (found.at(-1)?.title !== section) found.push({ title: section, entries: [] })
        found.at(-1)?.entries.push({ id, title })
      }
      setSections(found)
    }, 200)
    return () => clearTimeout(timer)
  }, [fetching, locale])
  return sections
}

// Every screen of the artifact on one page, in a fixed order, for a reader
// who does not open the console. The page is as wide as the printed sheet,
// so the charts draw at the size they print.
export function Report() {
  const { print } = useSearch({ from: '/$snapshotId/report' })
  const { range } = useRange()
  const contents = useContents()
  const locale = useLocale()
  const t = useT()
  usePrintWhenReady(print)
  // The page language, for hyphenation and screen readers. The console is
  // English again once the report closes.
  useEffect(() => {
    document.documentElement.lang = locale
    return () => {
      document.documentElement.lang = 'en'
    }
  }, [locale])
  return (
    <div className="report">
      <p className="muted">
        {range
          ? t('Charts cover {from} to {to}. Overview tiles cover the whole session.', {
              from: utcMinute(new Date(range.from).toISOString()),
              to: utcMinute(new Date(range.to).toISOString()),
            })
          : t('Charts cover the whole session.')}
      </p>
      <Section title="Contents" className="report-contents">
        {contents.map((section) => (
          <section key={section.title}>
            <h3 className="t-label">{section.title}</h3>
            <ol>
              {section.entries.map((entry) => (
                <li key={entry.id}>
                  <a href={`#${entry.id}`}>{entry.title}</a>
                </li>
              ))}
            </ol>
          </section>
        ))}
      </Section>
      <Metrics />
      <Section title="Queries">
        <Queries />
      </Section>
      <Section title="Logs">
        <NotCollected what="Logs" />
      </Section>
    </div>
  )
}
