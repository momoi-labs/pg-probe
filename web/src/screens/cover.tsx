import {
  Card,
  KV,
  KVKey,
  KVValue,
  Stat,
  StatFoot,
  StatLabel,
  StatValue,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@momoi-labs/kiso-react'
import { useEffect, type ReactNode } from 'react'
import type { Row, Sample, SnapshotListing } from '../api.ts'
import { num, sum, useServerVersion, useTable } from '../artifact.ts'
import { bytes, duration, Unknown } from '../format.tsx'
import { useLocale, useT, type Translate } from '../i18n.ts'
import { useRange } from '../range.tsx'
import lockup from '../assets/lockup.svg?inline'
import { PgProbeLockup } from './mark.tsx'

// What became of each source over the session: how many times it collected,
// and every other outcome grouped by status and error code.
type Missing = { status: string; errorCode: string | null; count: number }
type Source = { source: string; collected: number; missing: Missing[] }

export function coverage(outcomes: Row[]) {
  const sources = new Map<string, Source>()
  for (const outcome of outcomes) {
    const name = String(outcome.source)
    const source = sources.get(name) ?? { source: name, collected: 0, missing: [] }
    sources.set(name, source)
    if (outcome.status === 'collected') {
      source.collected += 1
      continue
    }
    const status = String(outcome.status)
    const errorCode = outcome.error_code == null ? null : String(outcome.error_code)
    const group = source.missing.find((item) => item.status === status && item.errorCode === errorCode)
    if (group) group.count += 1
    else source.missing.push({ status, errorCode, count: 1 })
  }
  const all = [...sources.values()]
  // A source that failed even once is failed. One the host or server lacks
  // is expected: in part when other databases or samples had it.
  const failed = all.filter((source) => source.missing.some((item) => item.status !== 'unsupported'))
  const expected = all.filter((source) => source.missing.length > 0 && !failed.includes(source))
  return {
    total: all.length,
    full: all.filter((source) => source.missing.length === 0).length,
    failed,
    partial: expected.filter((source) => source.collected > 0),
    unavailable: expected.filter((source) => source.collected === 0),
  }
}

type Coverage = ReturnType<typeof coverage>

const hasGaps = (found: Coverage) =>
  found.failed.length + found.partial.length + found.unavailable.length > 0

const hhmmss = (iso: string) => iso.slice(11, 19)
const hhmm = (iso: string) => iso.slice(11, 16)

// The most common time between samples, in whole seconds.
function interval(samples: Sample[]) {
  const steps = new Map<number, number>()
  for (let i = 1; i < samples.length; i++) {
    const step = Math.round((Date.parse(samples[i].taken_at_utc) - Date.parse(samples[i - 1].taken_at_utc)) / 1000)
    steps.set(step, (steps.get(step) ?? 0) + 1)
  }
  return [...steps.entries()].sort((a, b) => b[1] - a[1])[0]?.[0] ?? null
}

function bytesText(value: number) {
  const { number, unit } = bytes(value)
  return `${number} ${unit}`
}

// Why a source is not available, in words, from the error code the collector
// wrote. An unknown code is shown as it is.
function reason(errorCode: string | null, t: Translate, major: string | null, databases: string[]): ReactNode {
  switch (errorCode) {
    case 'not_in_server_version':
      return major
        ? t('PostgreSQL {version} does not have these statistics', { version: major })
        : t('This PostgreSQL version does not have these statistics')
    case 'psi_read_failed':
      return t('The kernel has no pressure stall information (PSI)')
    case 'cgroup_events_not_available':
      return t('The PostgreSQL cgroup has no events file')
    case 'cgroup_io_not_available':
      return t('The PostgreSQL cgroup has no I/O statistics')
    case 'thp_not_available':
      return t('The kernel has no transparent huge pages settings')
    case 'os_release_not_found':
      return t('The host has no /etc/os-release')
    case 'not_linux':
      return t('The capture ran on a host that is not Linux')
    case 'database_not_connectable':
      return databases.length > 0
        ? t('These databases do not accept connections: {names}', { names: databases.join(', ') })
        : t('Some databases do not accept connections')
    default:
      return <code>{errorCode ?? 'unsupported'}</code>
  }
}

function Fact({ name, children }: { name: string; children: ReactNode }) {
  const t = useT()
  return (
    <>
      <KVKey>{t(name)}</KVKey>
      <KVValue>{children ?? <Unknown reason={t('not recorded')} />}</KVValue>
    </>
  )
}

function CoverTile({ label, value, foot }: { label: string; value: ReactNode; foot: ReactNode }) {
  return (
    <Card>
      <Stat>
        <StatLabel>{label}</StatLabel>
        <StatValue>{value}</StatValue>
        <StatFoot>{foot}</StatFoot>
      </Stat>
    </Card>
  )
}

type CoverProps = { snapshot: SnapshotListing; outcomes: Row[]; samples: Sample[] | undefined }

// The first page of the printed report: the server, when it was captured,
// what came in, and the facts a reader needs before the charts. It also sets
// the running header of the pages after it.
export function ReportCover({ snapshot, outcomes, samples }: CoverProps) {
  const t = useT()
  const locale = useLocale()
  const { range } = useRange()
  const target = useTable('postgres_target').data?.[0]
  const identity = useTable('linux_identity').data?.[0]
  const cpus = useTable('linux_cpu_set').data
  const memory = useTable('linux_memory').data
  const databases = useTable('postgres_database').data
  const host = snapshot.hostname ?? snapshot.file_name
  const start = snapshot.started_at_utc
  const end = snapshot.finished_at_utc
  const date = start.slice(0, 10)
  const weekday = new Intl.DateTimeFormat(locale, { weekday: 'long', timeZone: 'UTC' }).format(new Date(start))
  const running = `${host} · ${t('{date}, {from} to {to} UTC', { date, from: hhmm(start), to: hhmm(end) })}`

  // The page margin boxes read these. The logo is inline, so it has loaded
  // by the time the page prints.
  useEffect(() => {
    const root = document.documentElement.style
    root.setProperty('--report-running', JSON.stringify(running))
    root.setProperty('--report-logo', `url(${JSON.stringify(lockup)})`)
    return () => {
      root.removeProperty('--report-running')
      root.removeProperty('--report-logo')
    }
  }, [running])

  const found = coverage(outcomes)
  const count = samples?.length ?? 0
  const step = samples ? interval(samples) : null
  const online = cpus?.filter((row) => row.set_kind === 'online').length ?? 0
  const memTotal = num(memory?.find((row) => row.key === 'MemTotal'), 'value_bytes')
  const clusterSize = databases?.some((row) => num(row, 'size_bytes') != null)
    ? sum(databases, 'size_bytes')
    : null
  const tool = outcomes.find((row) => row.collector_version != null)?.collector_version

  let sources: { value: string; foot: string }
  if (found.failed.length > 0) {
    sources = {
      value:
        found.failed.length === 1
          ? t('1 source failed')
          : t('{count} sources failed', { count: found.failed.length }),
      foot: t('{full} of {total} collected in full. See page 2.', { full: found.full, total: found.total }),
    }
  } else if (hasGaps(found)) {
    sources = {
      value: t('{collected} of {total} sources', {
        collected: found.full + found.partial.length,
        total: found.total,
      }),
      foot:
        found.partial.length > 0
          ? t('Collected, {count} of them in part. See page 2.', { count: found.partial.length })
          : t('Collected. See page 2.'),
    }
  } else {
    sources = { value: t('All {total} sources', { total: found.total }), foot: t('Collected in full.') }
  }

  return (
    <header className="report-cover">
      <div className="report-cover-band">
        <PgProbeLockup className="report-cover-logo" />
        <p className="report-kicker">{t('PostgreSQL capture report')}</p>
        <h1>{host}</h1>
        <p className="report-cover-period">
          {weekday}, {date} · {t('{from} to {to} UTC', { from: hhmmss(start), to: hhmmss(end) })}
        </p>
      </div>
      <div className="report-cover-body">
        <div className="report-cover-tiles">
          <CoverTile
            label={t('Duration')}
            value={duration(start, end) ?? '—'}
            foot={
              step == null
                ? count === 1
                  ? t('1 sample')
                  : t('{count} samples', { count })
                : t('{count} samples, every {step} s', { count, step })
            }
          />
          <CoverTile
            label={t('Status')}
            value={snapshot.status === 'complete' ? t('Complete') : t('Partial')}
            foot={
              snapshot.status === 'complete'
                ? t('The capture ran to the end.')
                : t('The capture stopped early, or a source failed.')
            }
          />
          <CoverTile label={t('Sources')} value={sources.value} foot={sources.foot} />
        </div>
        <div className="report-cover-facts">
          <section>
            <h2 className="t-label">{t('Host')}</h2>
            <KV>
              <Fact name="System">{identity?.distribution_pretty_name}</Fact>
              <Fact name="Kernel">
                {identity &&
                  `${identity.kernel_sysname} ${identity.kernel_release}, ${identity.kernel_machine}`}
              </Fact>
              <Fact name="CPUs">{online > 0 ? online : null}</Fact>
              <Fact name="Memory">{memTotal != null ? bytesText(memTotal) : null}</Fact>
            </KV>
          </section>
          <section>
            <h2 className="t-label">PostgreSQL</h2>
            <KV>
              <Fact name="Version">
                {target &&
                  `${target.server_version}, ${target.in_recovery ? t('standby') : t('primary')}`}
              </Fact>
              <Fact name="Cluster size">{clusterSize != null ? bytesText(clusterSize) : null}</Fact>
              <Fact name="Connected as">
                {target &&
                  (target.is_superuser
                    ? t('{user}, superuser', { user: String(target.connected_user) })
                    : target.connected_user)}
              </Fact>
            </KV>
          </section>
          <section>
            <h2 className="t-label">{t('Capture')}</h2>
            <KV>
              <Fact name="Tool">{tool != null ? `pg-probe ${tool}` : null}</Fact>
              <Fact name="Endpoint">
                {target &&
                  (target.endpoint_kind === 'tcp'
                    ? `${target.endpoint}:${target.port}`
                    : target.endpoint)}
              </Fact>
              <Fact name="Charts cover">
                {range
                  ? t('{from} to {to} UTC', {
                      from: hhmm(new Date(range.from).toISOString()),
                      to: hhmm(new Date(range.to).toISOString()),
                    })
                  : t('The whole session')}
              </Fact>
            </KV>
          </section>
        </div>
      </div>
    </header>
  )
}

function SourceNames({ sources }: { sources: Source[] }) {
  return (
    <>
      {sources.map((source, index) => (
        <span key={source.source}>
          {index > 0 && <br />}
          <code>{source.source}</code>
        </span>
      ))}
    </>
  )
}

// The page after the cover, when a source did not collect in full: what
// failed, what came in part, and what this host or server does not have.
export function DataCoverage({ outcomes, samples }: { outcomes: Row[]; samples: Sample[] | undefined }) {
  const t = useT()
  const { major } = useServerVersion()
  const databases = useTable('postgres_database').data ?? []
  const found = coverage(outcomes)
  if (!hasGaps(found)) return null
  const closed = databases
    .filter((row) => row.extension_visit_status === 'not_connectable')
    .map((row) => String(row.database_name))
  const why = (code: string | null) => reason(code, t, major, closed)
  const every = samples?.length

  // The sources this host or server lacks, by reason.
  const byReason = new Map<string, { code: string | null; sources: Source[] }>()
  for (const source of found.unavailable) {
    const code = source.missing[0]?.errorCode ?? null
    const key = String(code)
    const group = byReason.get(key) ?? { code, sources: [] }
    group.sources.push(source)
    byReason.set(key, group)
  }

  return (
    <section className="report-coverage" aria-labelledby="data-coverage">
      <h2 id="data-coverage">{t('Data coverage')}</h2>
      <p className="muted">
        {t('pg-probe read {total} sources. {full} collected in full. Charts show the rest as unknown, never as zero.', {
          total: found.total,
          full: found.full,
        })}
      </p>
      {found.failed.length > 0 && (
        <section>
          <h3>{t('Failed')}</h3>
          <p className="muted">
            {t('The source exists on this host, but pg-probe could not read it. Fix the cause and capture again.')}
          </p>
          <Table density="compact">
            <TableHeader>
              <TableRow>
                <TableHead>{t('Source')}</TableHead>
                <TableHead>{t('Error')}</TableHead>
                <TableHead>{t('Samples')}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {found.failed.flatMap((source) =>
                source.missing
                  .filter((item) => item.status !== 'unsupported')
                  .map((item) => (
                    <TableRow key={`${source.source}|${item.status}|${item.errorCode}`}>
                      <TableCell>
                        <code>{source.source}</code>
                      </TableCell>
                      <TableCell>
                        <code>{item.errorCode ?? item.status}</code>
                      </TableCell>
                      <TableCell>
                        {every != null && every > 1 && item.count === every
                          ? t('every sample')
                          : t('{count} of {total}', { count: item.count, total: every ?? item.count })}
                      </TableCell>
                    </TableRow>
                  )),
              )}
            </TableBody>
          </Table>
        </section>
      )}
      {found.partial.length > 0 && (
        <section>
          <h3>{t('Collected in part')}</h3>
          <p className="muted">{t('Some of the data came in. The rest is expected to be missing.')}</p>
          <Table density="compact">
            <TableHeader>
              <TableRow>
                <TableHead>{t('Source')}</TableHead>
                <TableHead>{t('Why')}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {found.partial.map((source) => (
                <TableRow key={source.source}>
                  <TableCell>
                    <code>{source.source}</code>
                  </TableCell>
                  <TableCell>{why(source.missing[0]?.errorCode ?? null)}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </section>
      )}
      {found.unavailable.length > 0 && (
        <section>
          <h3>{t('Not available on this host or version')}</h3>
          <p className="muted">{t('This kernel or server version does not have these sources. Nothing to fix.')}</p>
          <Table density="compact">
            <TableHeader>
              <TableRow>
                <TableHead>{t('Reason')}</TableHead>
                <TableHead>{t('Sources')}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {[...byReason.values()].map((group) => (
                <TableRow key={String(group.code)}>
                  <TableCell>{why(group.code)}</TableCell>
                  <TableCell>
                    <SourceNames sources={group.sources} />
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </section>
      )}
    </section>
  )
}
