import {
  Alert,
  AlertContent,
  AlertDescription,
  AlertTitle,
  Button,
  Disclosure,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
  PageHeader,
  PageHeaderDescription,
  PageHeaderTitle,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Skeleton,
  StatusBadge,
} from '@momoi-labs/kiso-react'
import { useQuery } from '@tanstack/react-query'
import { Link, Outlet, useNavigate, useParams } from '@tanstack/react-router'
import { ChevronDownIcon, PrinterIcon } from 'lucide-react'
import {
  routeId,
  samplesQuery,
  snapshotQuery,
  snapshotsQuery,
  tableQuery,
  type Row,
} from '../api.ts'
import { duration, utcMinute } from '../format.tsx'
import { RangePicker, RangeProvider, runBounds } from '../range.tsx'
import { localeNames, locales, useT } from '../i18n.ts'
import { useInReport } from '../report.ts'
import { DataCoverage, ReportCover } from './cover.tsx'

function ArtifactPicker({ current }: { current: string }) {
  const navigate = useNavigate()
  const { data: snapshots } = useQuery(snapshotsQuery)
  if (!snapshots) return null
  return (
    <Select
      value={current}
      onValueChange={(snapshotId) => navigate({ to: '/$snapshotId', params: { snapshotId } })}
    >
      <SelectTrigger aria-label="Artifact">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {snapshots.map((listing) => (
          <SelectItem key={listing.id} value={routeId(listing)}>
            {listing.hostname ?? listing.file_name} · {utcMinute(listing.started_at_utc)}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

type OutcomeGroup = { source: string; status: string; errorCode: string | null; count: number }

// Group repeated outcomes, for example one extension visit per database.
function notCollected(outcomes: Row[]): OutcomeGroup[] {
  const groups = new Map<string, OutcomeGroup>()
  for (const outcome of outcomes) {
    if (outcome.status === 'collected') continue
    const source = String(outcome.source)
    const status = String(outcome.status)
    const errorCode = outcome.error_code == null ? null : String(outcome.error_code)
    const key = `${source}|${status}|${errorCode}`
    const group = groups.get(key) ?? { source, status, errorCode, count: 0 }
    group.count += 1
    groups.set(key, group)
  }
  return [...groups.values()]
}

// Opens the printed report in the language the reader picks.
function PrintReport({ snapshotId }: { snapshotId: string }) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button>
          <PrinterIcon aria-hidden />
          Print report
          <ChevronDownIcon aria-hidden />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuLabel>Report language</DropdownMenuLabel>
        {locales.map((locale) => (
          <DropdownMenuItem key={locale} asChild>
            <Link
              to="/$snapshotId/report"
              params={{ snapshotId }}
              search={{ print: true, lang: locale === 'en-US' ? undefined : locale }}
              lang={locale}
            >
              {localeNames[locale]}
            </Link>
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function Subtitle({ id }: { id: string }) {
  const t = useT()
  const target = useQuery(tableQuery(id, 'postgres_target')).data?.[0]
  const identity = useQuery(tableQuery(id, 'linux_identity')).data?.[0]
  const parts = [
    target?.server_version != null && `PostgreSQL ${target.server_version}`,
    identity?.distribution_pretty_name != null &&
      t('on {distribution}', { distribution: identity.distribution_pretty_name }),
  ].filter(Boolean)
  return <>{parts.join(' ')}</>
}

export function SnapshotLayout() {
  const { snapshotId } = useParams({ from: '/$snapshotId' })
  const { data, error, isPending } = useQuery(snapshotQuery(snapshotId))
  const samples = useQuery(samplesQuery(snapshotId)).data
  const sampleCount = samples?.length
  const bounds = runBounds(samples)
  const inReport = useInReport()
  const t = useT()

  if (isPending) {
    return (
      <PageHeader>
        <Skeleton variant="text" />
      </PageHeader>
    )
  }

  if (error) {
    return (
      <>
        <PageHeader actions={<ArtifactPicker current={snapshotId} />}>
          <PageHeaderTitle>{snapshotId}</PageHeaderTitle>
        </PageHeader>
        <Alert variant="error">
          <AlertContent>
            <AlertTitle>Could not open this artifact</AlertTitle>
            <AlertDescription>
              {error.message}. Pick another artifact, or check that the file is a
              complete <code>.sqlite.gz</code> written by <code>pg-probe capture</code>.
            </AlertDescription>
          </AlertContent>
        </Alert>
      </>
    )
  }

  const { snapshot, collector_outcomes: outcomes } = data
  const missing = notCollected(outcomes)
  // Failures can change how the rest of the page reads, so they stay in view.
  // Sources this host or server version lacks are expected; each panel that
  // needs one already says so, and the full list stays one click away.
  const failed = missing.filter((group) => group.status !== 'unsupported')
  const unsupported = missing.filter((group) => group.status === 'unsupported')
  const took = duration(snapshot.started_at_utc, snapshot.finished_at_utc)
  // Distinct sources, not one per sample: a source that is unsupported in a
  // 19-sample session is one missing source.
  const distinct = (groups: OutcomeGroup[]) => new Set(groups.map((group) => group.source)).size
  const sourceList = (groups: OutcomeGroup[]) => (
    <ul>
      {groups.map((group) => (
        <li key={`${group.source}|${group.status}|${group.errorCode}`}>
          <code>{group.source}</code>: {t(group.status)}
          {group.errorCode && (
            <>
              {' '}
              (<code>{group.errorCode}</code>)
            </>
          )}
          {sampleCount != null && sampleCount > 1 && group.count === sampleCount
            ? t(', every sample')
            : group.count > 1 && ` ×${group.count}`}
        </li>
      ))}
    </ul>
  )

  const unsupportedSummary =
    distinct(unsupported) === 1
      ? t('1 source is not available on this host or server version')
      : t('{count} sources are not available on this host or server version', {
          count: distinct(unsupported),
        })

  // One RangeProvider for the console and the report, so the range a reader
  // picked holds when the report opens. The printed report opens on its own
  // cover, and on a page that lists the sources that did not collect in full.
  return (
    <RangeProvider artifact={snapshotId}>
      {inReport ? (
        <>
          <ReportCover snapshot={snapshot} outcomes={outcomes} samples={samples} />
          <DataCoverage outcomes={outcomes} samples={samples} />
        </>
      ) : (
        <>
          <PageHeader
            actions={
              <>
                {bounds && <RangePicker bounds={bounds} />}
                <StatusBadge tone={snapshot.status === 'complete' ? 'success' : 'neutral'}>
                  {snapshot.status}
                </StatusBadge>
                <ArtifactPicker current={snapshotId} />
                <PrintReport snapshotId={snapshotId} />
              </>
            }
          >
            <PageHeaderTitle>{snapshot.hostname ?? snapshot.file_name}</PageHeaderTitle>
            <PageHeaderDescription>
              <Subtitle id={snapshotId} /> ·{' '}
              {took
                ? t('captured {time} in {duration}', { time: utcMinute(snapshot.started_at_utc), duration: took })
                : t('captured {time}', { time: utcMinute(snapshot.started_at_utc) })}
              {sampleCount != null &&
                ` · ${sampleCount === 1 ? t('1 sample') : t('{count} samples', { count: sampleCount })}`}
            </PageHeaderDescription>
          </PageHeader>
          {failed.length > 0 && (
            <Alert variant="warning">
              <AlertContent>
                <AlertTitle>
                  {distinct(failed) === 1
                    ? t('1 source failed')
                    : t('{count} sources failed', { count: distinct(failed) })}
                </AlertTitle>
                <AlertDescription>
                  {t(
                    'These sources have no data in this artifact. Panels that read them show an unknown value.',
                  )}
                </AlertDescription>
                {sourceList(failed)}
              </AlertContent>
            </Alert>
          )}
          {unsupported.length > 0 && (
            <Disclosure summary={unsupportedSummary}>{sourceList(unsupported)}</Disclosure>
          )}
        </>
      )}
      <Outlet />
    </RangeProvider>
  )
}
