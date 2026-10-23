import { Disclosure } from '@momoi-labs/kiso-react'
import { useIsFetching } from '@tanstack/react-query'
import { useNavigate, useSearch } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import { utcMinute } from '../format.tsx'
import { useRange } from '../range.tsx'
import { Metrics } from './metrics.tsx'
import { NotCollected } from './not-collected.tsx'
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

type Section = { title: string; entries: { id: string; title: string }[] }

// The charts and tables of the report by section, read from the drawn page
// once everything has loaded, for the contents.
function useContents() {
  const fetching = useIsFetching()
  const [sections, setSections] = useState<Section[]>([])
  useEffect(() => {
    if (fetching > 0) return
    const timer = setTimeout(() => {
      const found: Section[] = []
      for (const pane of document.querySelectorAll<HTMLElement>('.report .grid-pane:has(figure, table)')) {
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
  }, [fetching])
  return sections
}

// Every screen of the artifact on one page, in a fixed order, for a reader
// who does not open the console. The page is as wide as the printed sheet,
// so the charts draw at the size they print.
export function Report() {
  const { print } = useSearch({ from: '/$snapshotId/report' })
  const { range } = useRange()
  const contents = useContents()
  usePrintWhenReady(print)
  return (
    <div className="report">
      <p className="muted">
        {range
          ? `Charts cover ${utcMinute(new Date(range.from).toISOString())} to ${utcMinute(new Date(range.to).toISOString())}. Overview tiles cover the whole session.`
          : 'Charts cover the whole session.'}
      </p>
      <Disclosure summary="Contents" open className="report-contents">
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
      </Disclosure>
      <Metrics />
      <Disclosure summary="Queries" open>
        <Queries />
      </Disclosure>
      <Disclosure summary="Logs" open>
        <NotCollected what="Logs" />
      </Disclosure>
    </div>
  )
}
