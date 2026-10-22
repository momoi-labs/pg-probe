import {
  Alert,
  AlertContent,
  AlertDescription,
  AlertTitle,
  Disclosure,
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
import { Outlet, useNavigate, useParams } from '@tanstack/react-router'
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

function Subtitle({ id }: { id: string }) {
  const target = useQuery(tableQuery(id, 'postgres_target')).data?.[0]
  const identity = useQuery(tableQuery(id, 'linux_identity')).data?.[0]
  const parts = [
    target?.server_version != null && `PostgreSQL ${target.server_version}`,
    identity?.distribution_pretty_name != null && `on ${identity.distribution_pretty_name}`,
  ].filter(Boolean)
  return <>{parts.join(' ')}</>
}

export function SnapshotLayout() {
  const { snapshotId } = useParams({ from: '/$snapshotId' })
  const { data, error, isPending } = useQuery(snapshotQuery(snapshotId))
  const samples = useQuery(samplesQuery(snapshotId)).data
  const sampleCount = samples?.length
  const bounds = runBounds(samples)

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
          <code>{group.source}</code>: {group.status}
          {group.errorCode && (
            <>
              {' '}
              (<code>{group.errorCode}</code>)
            </>
          )}
          {sampleCount != null && sampleCount > 1 && group.count === sampleCount
            ? ', every sample'
            : group.count > 1 && ` ×${group.count}`}
        </li>
      ))}
    </ul>
  )

  return (
    <RangeProvider artifact={snapshotId}>
      <PageHeader
        actions={
          <>
            {bounds && <RangePicker bounds={bounds} />}
            <StatusBadge tone={snapshot.status === 'complete' ? 'success' : 'neutral'}>
              {snapshot.status}
            </StatusBadge>
            <ArtifactPicker current={snapshotId} />
          </>
        }
      >
        <PageHeaderTitle>{snapshot.hostname ?? snapshot.file_name}</PageHeaderTitle>
        <PageHeaderDescription>
          <Subtitle id={snapshotId} /> · captured {utcMinute(snapshot.started_at_utc)}
          {took && ` in ${took}`}
          {sampleCount != null && ` · ${sampleCount} ${sampleCount === 1 ? 'sample' : 'samples'}`}
        </PageHeaderDescription>
      </PageHeader>
      {failed.length > 0 && (
        <Alert variant="warning">
          <AlertContent>
            <AlertTitle>
              {distinct(failed) === 1
                ? '1 source failed'
                : `${distinct(failed)} sources failed`}
            </AlertTitle>
            <AlertDescription>
              These sources have no data in this artifact. Panels that read them show an
              unknown value.
            </AlertDescription>
            {sourceList(failed)}
          </AlertContent>
        </Alert>
      )}
      {unsupported.length > 0 && (
        <Disclosure
          summary={`${distinct(unsupported)} ${distinct(unsupported) === 1 ? 'source is' : 'sources are'} not available on this host or server version`}
        >
          {sourceList(unsupported)}
        </Disclosure>
      )}
      <Outlet />
    </RangeProvider>
  )
}
