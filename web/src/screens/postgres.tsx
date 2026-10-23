import {
  BarGauge,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@momoi-labs/kiso-react'
import type { Row } from '../api.ts'
import {
  collected,
  latest,
  missingReason,
  num,
  sum,
  useOutcomes,
  useServerVersion,
  useTable,
} from '../artifact.ts'
import { bytes } from '../format.tsx'
import { useT } from '../i18n.ts'
import { deltas, points, rates, useSamples, type Values } from '../series.ts'
import { ChartPanel, Note, NotSupportedPanel, Panel, PanelGrid, Section, Slot } from './panels.tsx'

const MIB = 1024 ** 2
const oneDecimal = (value: number) => value.toFixed(1)
const twoDecimals = (value: number) => value.toFixed(2)

// Before PostgreSQL 10, pg_stat_activity lists only client backends, and the
// probe records them as client_backend. Older artifacts stored "<invalid>".
export const isClientBackend = (row: Row) =>
  row.backend_type == null ||
  row.backend_type === 'client_backend' ||
  row.backend_type === 'client backend' ||
  row.backend_type === '<invalid>'

const stateOf = (row: Row) => (row.state == null ? 'NULL' : String(row.state))

function ConnectionsByState() {
  const samples = useSamples()
  const activity = useTable('postgres_activity_aggregates')
  const clients = (activity.data ?? []).filter(isClientBackend)
  // Four busiest states by total count, the rest grouped, to stay in five series.
  const totals = new Map<string, number>()
  for (const row of clients) {
    totals.set(stateOf(row), (totals.get(stateOf(row)) ?? 0) + (num(row, 'count') ?? 0))
  }
  const ranked = [...totals].sort((a, b) => b[1] - a[1]).map(([state]) => state)
  const shown = ranked.length > 5 ? ranked.slice(0, 4) : ranked
  const grouped = ranked.length > 5
  const chart = points(samples.data ?? [], clients, (rows) => {
    const values: Values = Object.fromEntries(shown.map((state) => [state, 0]))
    if (grouped) values.other = 0
    for (const row of rows) {
      const state = stateOf(row)
      const key = shown.includes(state) ? state : 'other'
      values[key] = (values[key] ?? 0) + (num(row, 'count') ?? 0)
    }
    return values
  })
  const series = [
    ...shown.map((state) => ({ key: state, label: state })),
    ...(grouped ? [{ key: 'other', label: 'Other states' }] : []),
  ]
  return (
    <ChartPanel
      title="Connections by state"
      loading={activity.isPending || samples.isPending}
      label="Client connections by state"
      variant="stacked-area"
      min={0}
      formatValue={(value) => value.toFixed(0)}
      data={chart}
      series={series}
    />
  )
}

function useDatabaseStats() {
  const samples = useSamples()
  const stats = useTable('postgres_database_stats')
  return {
    samples: samples.data ?? [],
    rows: stats.data ?? [],
    isPending: samples.isPending || stats.isPending,
  }
}

const totals =
  (...columns: string[]) =>
  (rows: Row[]): Values =>
    Object.fromEntries(columns.map((column) => [column, rows.length ? sum(rows, column) : null]))

function CacheHitRatio() {
  const stats = useDatabaseStats()
  const chart = deltas(stats.samples, stats.rows, totals('blks_hit', 'blks_read'), (delta) => {
    const hit = delta.blks_hit
    const read = delta.blks_read
    if (hit == null || read == null || hit + read === 0) return { ratio: null }
    return { ratio: (hit / (hit + read)) * 100 }
  })
  return (
    <ChartPanel
      title="Cache hit ratio"
      loading={stats.isPending}
      label="Cache hit ratio per interval (%)"
      max={100}
      formatValue={oneDecimal}
      data={chart}
      series={[{ key: 'ratio', label: 'All databases' }]}
    />
  )
}

function Transactions() {
  const stats = useDatabaseStats()
  const chart = rates(stats.samples, stats.rows, totals('xact_commit', 'xact_rollback'))
  return (
    <ChartPanel
      title="Transactions"
      loading={stats.isPending}
      label="Transactions per second"
      min={0}
      formatValue={oneDecimal}
      data={chart}
      series={[
        { key: 'xact_commit', label: 'Commits' },
        { key: 'xact_rollback', label: 'Rollbacks' },
      ]}
    />
  )
}

function TemporaryFiles() {
  const stats = useDatabaseStats()
  const chart = rates(stats.samples, stats.rows, (rows) => {
    const bytesWritten = totals('temp_bytes')(rows).temp_bytes
    return { temp_bytes: bytesWritten == null ? null : bytesWritten / MIB }
  })
  return (
    <ChartPanel
      title="Temporary files written"
      loading={stats.isPending}
      label="Temporary files written (MiB/s)"
      min={0}
      formatValue={twoDecimals}
      data={chart}
      series={[{ key: 'temp_bytes', label: 'All databases' }]}
    />
  )
}

// WAL generated per second. pg_stat_wal.wal_bytes from PostgreSQL 14, else
// the change in the recorded WAL position, which works from 9.2.
export function useWalRate() {
  const samples = useSamples()
  const wal = useTable('postgres_wal_stats')
  const position = useTable('postgres_wal_position')
  const fromStats = (wal.data?.length ?? 0) > 0
  const rows = fromStats ? (wal.data ?? []) : (position.data ?? [])
  const column = fromStats ? 'wal_bytes' : 'wal_position_bytes'
  const chart = rates(samples.data ?? [], rows, (sampleRows) => ({
    wal: sampleRows.length ? (num(sampleRows[0], column) ?? 0) / MIB : null,
  }))
  const source = rows.length === 0 ? null : fromStats ? 'pg_stat_wal.wal_bytes' : 'the WAL position'
  return {
    chart,
    source,
    isPending: samples.isPending || wal.isPending || position.isPending,
  }
}

function WalAndCheckpoints() {
  const samples = useSamples()
  const target = useTable('postgres_target')
  const bgwriter = useTable('postgres_bgwriter_stats')
  const checkpointer = useTable('postgres_checkpointer_stats')
  const wal = useWalRate()
  const blockSize = num(target.data?.[0], 'block_size_bytes') ?? 8192
  const pages = (value: number | null) => (value == null ? null : (value * blockSize) / MIB)
  const sampleList = samples.data ?? []
  // From PostgreSQL 17 the checkpoint counter moved to pg_stat_checkpointer.
  const fromCheckpointer = (checkpointer.data?.length ?? 0) > 0
  const writers = rates(sampleList, bgwriter.data ?? [], (rows) => ({
    checkpoint: fromCheckpointer ? null : pages(num(rows[0], 'buffers_checkpoint')),
    bgwriter: pages(num(rows[0], 'buffers_clean')),
    backend: pages(num(rows[0], 'buffers_backend')),
  }))
  const checkpoints = rates(sampleList, checkpointer.data ?? [], (rows) => ({
    checkpoint: pages(num(rows[0], 'buffers_written')),
  }))
  const walRates = wal.chart
  const hasWal = wal.source != null
  const chart = writers.map((point, index) => ({
    timestamp: point.timestamp,
    values: {
      ...(hasWal ? { wal: walRates[index]?.values.wal ?? null } : {}),
      ...point.values,
      ...(fromCheckpointer ? { checkpoint: checkpoints[index]?.values.checkpoint ?? null } : {}),
    },
  }))
  const series = [
    ...(hasWal ? [{ key: 'wal', label: 'WAL generated' }] : []),
    { key: 'checkpoint', label: 'Checkpoint writes' },
    { key: 'bgwriter', label: 'Background writer' },
    { key: 'backend', label: 'Backend writes' },
  ]
  return (
    <ChartPanel
      title="WAL and checkpoint writes"
      loading={
        samples.isPending || bgwriter.isPending || checkpointer.isPending || wal.isPending
      }
      label="WAL and buffer writes (MiB/s)"
      min={0}
      formatValue={twoDecimals}
      data={chart}
      series={series}
    />
  )
}

function ReplicationLag() {
  const t = useT()
  const replication = useTable('postgres_replication_aggregates')
  const data = latest(replication.data)
  const isPending = replication.isPending
  const outcomes = useOutcomes()
  let content
  if (data.length) {
    content = (
      <Table density="compact">
        <TableHeader>
          <TableRow>
            <TableHead>{t('State')}</TableHead>
            <TableHead>{t('Sync state')}</TableHead>
            <TableHead className="num">{t('Standbys')}</TableHead>
            <TableHead className="num">{t('Max lag')}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {data.map((row, index) => {
            const lag = num(row, 'max_lag_bytes')
            const lagText = lag == null ? null : bytes(lag)
            return (
              <TableRow key={index}>
                <TableCell>{row.state ?? 'NULL'}</TableCell>
                <TableCell>{row.sync_state ?? 'NULL'}</TableCell>
                <TableCell className="num">{num(row, 'count') ?? 'NULL'}</TableCell>
                <TableCell className="num">{lagText ? `${lagText.number} ${lagText.unit}` : 'NULL'}</TableCell>
              </TableRow>
            )
          })}
        </TableBody>
      </Table>
    )
  } else if (collected(outcomes, 'postgres.aggregates.replication')) {
    content = <Note>{t('No standby was connected in the last sample.')}</Note>
  } else {
    const reason = missingReason(outcomes, 'postgres.aggregates.replication')
    content = (
      <Note>
        {reason
          ? t('Not recorded in this artifact: {reason}.', { reason })
          : t('Not recorded in this artifact.')}
      </Note>
    )
  }
  return (
    <Panel title="Replication lag" sub="last sample, from pg_stat_replication" loading={isPending}>
      {content}
    </Panel>
  )
}

const IEC = ['B', 'KiB', 'MiB', 'GiB']

function DatabaseSize() {
  const t = useT()
  const { data, isPending } = useTable('postgres_database')
  const sized = (data ?? [])
    .filter((row) => num(row, 'size_bytes') != null)
    .sort((a, b) => (num(b, 'size_bytes') ?? 0) - (num(a, 'size_bytes') ?? 0))
  const largest = num(sized[0], 'size_bytes') ?? 0
  // One unit for the whole comparison, chosen from the largest database and
  // capped at GiB so smaller databases do not all round to zero.
  const exponent = largest > 0 ? Math.min(Math.floor(Math.log(largest) / Math.log(1024)), 3) : 0
  const format = (value: number) =>
    `${(value / 1024 ** exponent).toLocaleString('en-US', { minimumFractionDigits: 1, maximumFractionDigits: 1 })} ${IEC[exponent]}`
  return (
    <Panel title="Database size" sub="point in time" loading={isPending}>
      {sized.length > 0 ? (
        <>
          <BarGauge
            label={t('Size per database')}
            labels={{ notCollected: t('Not collected'), empty: t('No measurements.') }}
            max={largest}
            formatValue={format}
            rows={sized.map((row) => ({
              key: String(row.database_name),
              label: String(row.database_name),
              value: num(row, 'size_bytes'),
            }))}
          />
        </>
      ) : (
        <Note>{t('Not recorded in this artifact.')}</Note>
      )}
    </Panel>
  )
}

const relationName = (row: Row) => `${row.database_name}.${row.schema_name}.${row.relation_name}`

function LargestTables() {
  const t = useT()
  const target = useTable('postgres_target')
  const { data, isPending } = useTable('postgres_relation')
  const blockSize = num(target.data?.[0], 'block_size_bytes') ?? 8192
  const pages = (row: Row) =>
    (num(row, 'heap_pages') ?? 0) + (num(row, 'index_pages') ?? 0) + (num(row, 'toast_pages') ?? 0)
  const largest = [...(data ?? [])].sort((a, b) => pages(b) - pages(a)).slice(0, 6)
  const top = largest.length ? pages(largest[0]) * blockSize : 0
  return (
    <Panel title="Largest tables" sub="heap, indexes and TOAST, top 6" loading={isPending}>
      {largest.length > 0 ? (
        <>
          <BarGauge
            label={t('Size per table (GiB)')}
            labels={{ notCollected: t('Not collected'), empty: t('No measurements.') }}
            max={top / 1024 ** 3}
            formatValue={(value) => `${value.toFixed(2)} GiB`}
            rows={largest.map((row) => ({
              key: relationName(row),
              label: relationName(row),
              value: (pages(row) * blockSize) / 1024 ** 3,
            }))}
          />
        </>
      ) : (
        <Note>{t('Not recorded in this artifact. Captures before schema version 3 have no relations.')}</Note>
      )}
    </Panel>
  )
}

function DeadTuples() {
  const t = useT()
  const { data, isPending } = useTable('postgres_relation')
  const rows = [...(data ?? [])]
    .filter((row) => (num(row, 'dead_tuples') ?? 0) > 0)
    .sort((a, b) => (num(b, 'dead_tuples') ?? 0) - (num(a, 'dead_tuples') ?? 0))
    .slice(0, 6)
  let content
  if (rows.length > 0) {
    content = (
      <>
        <Table density="compact">
          <TableHeader>
            <TableRow>
              <TableHead>{t('Table')}</TableHead>
              <TableHead className="num">{t('Dead tuples')}</TableHead>
              <TableHead className="num">{t('Live tuples')}</TableHead>
              <TableHead className="num">{t('Dead (%)')}</TableHead>
              <TableHead>{t('Last autovacuum')}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row) => {
              const dead = num(row, 'dead_tuples') ?? 0
              const live = num(row, 'live_tuples')
              return (
                <TableRow key={relationName(row)}>
                  <TableCell>
                    <code>{relationName(row)}</code>
                  </TableCell>
                  <TableCell className="num">{dead.toLocaleString('en-US')}</TableCell>
                  <TableCell className="num">
                    {live == null ? 'NULL' : live.toLocaleString('en-US')}
                  </TableCell>
                  <TableCell className="num">
                    {live == null || live + dead === 0
                      ? '—'
                      : ((dead / (live + dead)) * 100).toFixed(1)}
                  </TableCell>
                  <TableCell>{row.last_autovacuum ?? 'NULL'}</TableCell>
                </TableRow>
              )
            })}
          </TableBody>
        </Table>
      </>
    )
  } else if (isPending) {
    content = null
  } else if (data?.length) {
    content = <Note>{t('No dead tuples in the recorded tables.')}</Note>
  } else {
    content = (
      <Note>{t('Not recorded in this artifact. Captures before schema version 3 have no relations.')}</Note>
    )
  }
  return (
    <Panel title="Dead tuples" sub="vacuum backlog, top 6" loading={isPending}>
      {content}
    </Panel>
  )
}

// Counts: whole numbers, with one decimal only for axis steps between them.
const integerValue = (value: number) =>
  Number.isInteger(value) ? value.toFixed(0) : value.toFixed(1)
const minutes = (value: number) => (value / 60).toFixed(1)

// Whether the observed server is older than `since`. Without a PostgreSQL
// target nothing is claimed, and the panel shows its own "not recorded" note.
function useOlderThan(since: number) {
  const version = useServerVersion()
  return {
    older: version.versionNum != null && version.versionNum < since,
    on: `PostgreSQL ${version.major}`,
    isPending: version.isPending,
  }
}

// The four keys with the most `count` over the session, plus an Other group
// when there are more than five, so a chart stays within five series.
function topKeys(rows: Row[], keyOf: (row: Row) => string, column = 'count') {
  const totals = new Map<string, number>()
  for (const row of rows) {
    totals.set(keyOf(row), (totals.get(keyOf(row)) ?? 0) + (num(row, column) ?? 0))
  }
  const ranked = [...totals].sort((a, b) => b[1] - a[1]).map(([key]) => key)
  const grouped = ranked.length > 5
  const shown = grouped ? ranked.slice(0, 4) : ranked
  const read = (sampleRows: Row[]): Values => {
    const values: Values = Object.fromEntries(shown.map((key) => [key, 0]))
    if (grouped) values.other = 0
    for (const row of sampleRows) {
      const key = shown.includes(keyOf(row)) ? keyOf(row) : 'other'
      values[key] = (values[key] ?? 0) + (num(row, column) ?? 0)
    }
    return values
  }
  const series = [
    ...shown.map((key) => ({ key, label: key })),
    ...(grouped ? [{ key: 'other', label: 'Other' }] : []),
  ]
  return { read, series }
}

const isGranted = (row: Row) => row.granted === 1

function LockWaits() {
  const samples = useSamples()
  const locks = useTable('postgres_lock_aggregates')
  const waiting = (locks.data ?? []).filter((row) => !isGranted(row))
  const byType = topKeys(waiting, (row) => String(row.lock_type))
  // A session without waits keeps one flat series at zero.
  const series = byType.series.length ? byType.series : [{ key: 'waiting', label: 'Waiting' }]
  const chart = points(samples.data ?? [], locks.data ?? [], (rows) => {
    if (rows.length === 0) return Object.fromEntries(series.map(({ key }) => [key, null]))
    return byType.series.length ? byType.read(rows.filter((row) => !isGranted(row))) : { waiting: 0 }
  })
  return (
    <ChartPanel
      title="Lock waits"
      loading={locks.isPending || samples.isPending}
      label="Lock requests waiting, by lock type"
      variant="stacked-area"
      min={0}
      formatValue={integerValue}
      data={chart}
      series={locks.data?.length ? series : []}
    />
  )
}

function LockWaitAge() {
  const samples = useSamples()
  const locks = useTable('postgres_lock_aggregates')
  const version = useOlderThan(140000)
  const label = 'Longest lock wait (s)'
  if (version.older) {
    return (
      <NotSupportedPanel title="Longest lock wait" label={label} on={version.on}>
        pg_locks.waitstart exists from PostgreSQL 14.
      </NotSupportedPanel>
    )
  }
  const chart = points(samples.data ?? [], locks.data ?? [], (rows) => {
    const ages = rows
      .filter((row) => !isGranted(row))
      .map((row) => num(row, 'max_wait_age_seconds') ?? 0)
    return { wait: rows.length ? Math.max(0, ...ages) : null }
  })
  return (
    <ChartPanel
      title="Longest lock wait"
      loading={locks.isPending || samples.isPending || version.isPending}
      label={label}
      min={0}
      formatValue={integerValue}
      data={chart}
      series={locks.data?.length ? [{ key: 'wait', label: 'Longest wait' }] : []}
    />
  )
}

function LocksHeld() {
  const samples = useSamples()
  const locks = useTable('postgres_lock_aggregates')
  const granted = (locks.data ?? []).filter(isGranted)
  const byMode = topKeys(granted, (row) => String(row.lock_mode))
  const chart = points(samples.data ?? [], granted, byMode.read)
  return (
    <ChartPanel
      title="Locks held"
      loading={locks.isPending || samples.isPending}
      label="Locks held, by mode"
      variant="stacked-area"
      min={0}
      formatValue={integerValue}
      data={chart}
      series={byMode.series}
    />
  )
}

const LONG_STATES = ['active', 'idle in transaction', 'idle in transaction (aborted)']

function TimeInState() {
  const samples = useSamples()
  const activity = useTable('postgres_activity_aggregates')
  const clients = (activity.data ?? []).filter(isClientBackend)
  const present = LONG_STATES.filter((state) => clients.some((row) => row.state === state))
  const chart = points(samples.data ?? [], clients, (rows) =>
    Object.fromEntries(
      present.map((state) => {
        const ages = rows
          .filter((row) => row.state === state)
          .map((row) => num(row, 'max_age_seconds'))
          .filter((age) => age != null)
        return [state, ages.length ? Math.max(...ages) : null]
      }),
    ),
  )
  return (
    <ChartPanel
      title="Longest time in state"
      loading={activity.isPending || samples.isPending}
      label="Longest time in state since state_change (min)"
      min={0}
      formatValue={minutes}
      data={chart}
      series={present.map((state) => ({ key: state, label: state }))}
    />
  )
}

function WaitEvents() {
  const samples = useSamples()
  const activity = useTable('postgres_activity_aggregates')
  const version = useOlderThan(90600)
  const label = 'Active backends, by wait event type'
  if (version.older) {
    return (
      <NotSupportedPanel title="Wait events" label={label} on={version.on}>
        pg_stat_activity.wait_event_type exists from PostgreSQL 9.6.
      </NotSupportedPanel>
    )
  }
  const active = (activity.data ?? []).filter(
    (row) => isClientBackend(row) && row.state === 'active',
  )
  const waitOf = (row: Row) => (row.wait_event_type == null ? 'Not waiting' : String(row.wait_event_type))
  const byWait = topKeys(active, waitOf)
  const chart = points(samples.data ?? [], active, byWait.read)
  return (
    <ChartPanel
      title="Wait events"
      loading={activity.isPending || samples.isPending || version.isPending}
      label={label}
      variant="stacked-area"
      min={0}
      formatValue={integerValue}
      data={chart}
      series={byWait.series}
    />
  )
}

function DatabaseErrors() {
  const stats = useDatabaseStats()
  const version = useOlderThan(120000)
  const columns = version.older ? ['deadlocks', 'conflicts'] : ['deadlocks', 'conflicts', 'checksum_failures']
  const chart = deltas(stats.samples, stats.rows, totals(...columns), (delta) => delta)
  return (
    <ChartPanel
      title="Deadlocks and conflicts"
      loading={stats.isPending || version.isPending}
      label={
        version.older
          ? 'Deadlocks and recovery conflicts per interval'
          : 'Deadlocks, recovery conflicts and checksum failures per interval'
      }
      min={0}
      formatValue={integerValue}
      data={chart}
      series={
        stats.rows.length
          ? [
              { key: 'deadlocks', label: 'Deadlocks' },
              { key: 'conflicts', label: 'Recovery conflicts' },
              ...(version.older ? [] : [{ key: 'checksum_failures', label: 'Checksum failures' }]),
            ]
          : []
      }
    />
  )
}

function SessionEnds() {
  const stats = useDatabaseStats()
  const version = useOlderThan(140000)
  const label = 'Sessions ended abnormally per interval'
  if (version.older) {
    return (
      <NotSupportedPanel title="Sessions ended abnormally" label={label} on={version.on}>
        The session counters of pg_stat_database exist from PostgreSQL 14.
      </NotSupportedPanel>
    )
  }
  const chart = deltas(
    stats.samples,
    stats.rows,
    totals('sessions_abandoned', 'sessions_fatal', 'sessions_killed'),
    (delta) => delta,
  )
  return (
    <ChartPanel
      title="Sessions ended abnormally"
      loading={stats.isPending || version.isPending}
      label={label}
      min={0}
      formatValue={integerValue}
      data={chart}
      series={
        stats.rows.length
          ? [
              { key: 'sessions_abandoned', label: 'Abandoned' },
              { key: 'sessions_fatal', label: 'Fatal' },
              { key: 'sessions_killed', label: 'Killed' },
            ]
          : []
      }
    />
  )
}

function BlockIoTime() {
  const stats = useDatabaseStats()
  const recorded = stats.rows.some((row) => num(row, 'blk_read_time_ms') != null)
  const chart = rates(stats.samples, stats.rows, (rows) => {
    const values = totals('blk_read_time_ms', 'blk_write_time_ms')(rows)
    return {
      read: recorded ? values.blk_read_time_ms : null,
      write: recorded ? values.blk_write_time_ms : null,
    }
  })
  return (
    <ChartPanel
      title="Block I/O time"
      loading={stats.isPending}
      label="Time reading and writing blocks (ms per second)"
      min={0}
      formatValue={oneDecimal}
      data={chart}
      series={
        recorded
          ? [
              { key: 'read', label: 'Read' },
              { key: 'write', label: 'Write' },
            ]
          : []
      }
    />
  )
}

function StandbyConflicts() {
  const samples = useSamples()
  const conflicts = useTable('postgres_standby_conflicts')
  const kinds = ['confl_snapshot', 'confl_lock', 'confl_bufferpin', 'confl_deadlock', 'confl_tablespace']
  const chart = deltas(samples.data ?? [], conflicts.data ?? [], totals(...kinds), (delta) => delta)
  return (
    <ChartPanel
      title="Standby conflicts"
      loading={conflicts.isPending || samples.isPending}
      label="Queries canceled by recovery conflicts per interval"
      min={0}
      formatValue={integerValue}
      data={chart}
      series={
        conflicts.data?.length
          ? [
              { key: 'confl_snapshot', label: 'Snapshot' },
              { key: 'confl_lock', label: 'Lock' },
              { key: 'confl_bufferpin', label: 'Buffer pin' },
              { key: 'confl_deadlock', label: 'Deadlock' },
              { key: 'confl_tablespace', label: 'Tablespace' },
            ]
          : []
      }
    />
  )
}

function WalArchiving() {
  const samples = useSamples()
  const archiver = useTable('postgres_archiver_stats')
  const version = useOlderThan(90400)
  const label = 'WAL files archived per interval'
  if (version.older) {
    return (
      <NotSupportedPanel title="WAL archiving" label={label} on={version.on}>
        pg_stat_archiver exists from PostgreSQL 9.4.
      </NotSupportedPanel>
    )
  }
  const chart = deltas(
    samples.data ?? [],
    archiver.data ?? [],
    (rows) => ({
      archived: num(rows[0], 'archived_count'),
      failed: num(rows[0], 'failed_count'),
    }),
    (delta) => delta,
  )
  return (
    <ChartPanel
      title="WAL archiving"
      loading={archiver.isPending || samples.isPending || version.isPending}
      label={label}
      min={0}
      formatValue={integerValue}
      data={chart}
      series={
        archiver.data?.length
          ? [
              { key: 'archived', label: 'Archived' },
              { key: 'failed', label: 'Failed' },
            ]
          : []
      }
    />
  )
}

function ReplicationSlots() {
  const t = useT()
  const samples = useSamples()
  const slots = useTable('postgres_slot_aggregates')
  const version = useOlderThan(90400)
  const label = 'WAL retained by replication slots (MiB)'
  if (version.older) {
    return (
      <NotSupportedPanel title="Replication slots" label={label} on={version.on}>
        Replication slots exist from PostgreSQL 9.4.
      </NotSupportedPanel>
    )
  }
  const types = [...new Set((slots.data ?? []).map((row) => String(row.slot_type)))].slice(0, 5)
  const chart = points(samples.data ?? [], slots.data ?? [], (rows) =>
    Object.fromEntries(
      types.map((type) => {
        const retained = rows
          .filter((row) => row.slot_type === type)
          .map((row) => num(row, 'max_retained_bytes'))
          .filter((value) => value != null)
        return [type, retained.length ? Math.max(...retained) / MIB : null]
      }),
    ),
  )
  return (
    <ChartPanel
      title="Replication slots"
      loading={slots.isPending || samples.isPending || version.isPending}
      label={label}
      min={0}
      formatValue={oneDecimal}
      data={chart}
      series={types.map((type) => ({ key: type, label: t('Largest {type} slot', { type }) }))}
    />
  )
}

function SlruReads() {
  const samples = useSamples()
  const slru = useTable('postgres_slru_stats')
  const version = useOlderThan(130000)
  const label = 'SLRU blocks read from disk per second'
  if (version.older) {
    return (
      <NotSupportedPanel title="SLRU reads" label={label} on={version.on}>
        pg_stat_slru exists from PostgreSQL 13.
      </NotSupportedPanel>
    )
  }
  const byName = topKeys(slru.data ?? [], (row) => String(row.name), 'blocks_read')
  const chart = rates(samples.data ?? [], slru.data ?? [], (rows) =>
    rows.length ? byName.read(rows) : Object.fromEntries(byName.series.map(({ key }) => [key, null])),
  )
  return (
    <ChartPanel
      title="SLRU reads"
      loading={slru.isPending || samples.isPending || version.isPending}
      label={label}
      min={0}
      formatValue={oneDecimal}
      data={chart}
      series={byName.series}
    />
  )
}

function IoByOperation() {
  const samples = useSamples()
  const io = useTable('postgres_io_stats')
  const version = useOlderThan(160000)
  const label = 'I/O by operation, all backends (MiB/s)'
  if (version.older) {
    return (
      <NotSupportedPanel title="I/O by operation" label={label} on={version.on}>
        pg_stat_io exists from PostgreSQL 16.
      </NotSupportedPanel>
    )
  }
  const operationBytes = (rows: Row[], column: string) =>
    rows.reduce((total, row) => total + (num(row, column) ?? 0) * (num(row, 'op_bytes') ?? 0), 0) / MIB
  const chart = rates(samples.data ?? [], io.data ?? [], (rows) =>
    rows.length
      ? {
          reads: operationBytes(rows, 'reads'),
          writes: operationBytes(rows, 'writes'),
          extends: operationBytes(rows, 'extends'),
        }
      : { reads: null, writes: null, extends: null },
  )
  return (
    <ChartPanel
      title="I/O by operation"
      loading={io.isPending || samples.isPending || version.isPending}
      label={label}
      min={0}
      formatValue={twoDecimals}
      data={chart}
      series={
        io.data?.length
          ? [
              { key: 'reads', label: 'Reads' },
              { key: 'writes', label: 'Writes' },
              { key: 'extends', label: 'Extends' },
            ]
          : []
      }
    />
  )
}

export function Postgres() {
  return (
    <Section title="PostgreSQL">
      <PanelGrid id="postgres" label="PostgreSQL panels">
        <Slot id="connections-by-state" size={8}>
          <ConnectionsByState />
        </Slot>
        <Slot id="cache-hit-ratio" size={4}>
          <CacheHitRatio />
        </Slot>
        <Slot id="transactions" size={6}>
          <Transactions />
        </Slot>
        <Slot id="temporary-files-written" size={6}>
          <TemporaryFiles />
        </Slot>
        <Slot id="wal-and-checkpoint-writes" size={6}>
          <WalAndCheckpoints />
        </Slot>
        <Slot id="replication-lag" size={6}>
          <ReplicationLag />
        </Slot>
        <Slot id="database-size" size={6}>
          <DatabaseSize />
        </Slot>
        <Slot id="largest-tables" size={6}>
          <LargestTables />
        </Slot>
        <Slot id="dead-tuples" size={12}>
          <DeadTuples />
        </Slot>
        <Slot id="lock-waits" size={6}>
          <LockWaits />
        </Slot>
        <Slot id="longest-lock-wait" size={6}>
          <LockWaitAge />
        </Slot>
        <Slot id="locks-held" size={6}>
          <LocksHeld />
        </Slot>
        <Slot id="longest-time-in-state" size={6}>
          <TimeInState />
        </Slot>
        <Slot id="wait-events" size={6}>
          <WaitEvents />
        </Slot>
        <Slot id="deadlocks-and-conflicts" size={6}>
          <DatabaseErrors />
        </Slot>
        <Slot id="sessions-ended-abnormally" size={6}>
          <SessionEnds />
        </Slot>
        <Slot id="block-i-o-time" size={6}>
          <BlockIoTime />
        </Slot>
        <Slot id="standby-conflicts" size={6}>
          <StandbyConflicts />
        </Slot>
        <Slot id="wal-archiving" size={6}>
          <WalArchiving />
        </Slot>
        <Slot id="replication-slots" size={6}>
          <ReplicationSlots />
        </Slot>
        <Slot id="slru-reads" size={6}>
          <SlruReads />
        </Slot>
        <Slot id="i-o-by-operation" size={6}>
          <IoByOperation />
        </Slot>
      </PanelGrid>
    </Section>
  )
}
