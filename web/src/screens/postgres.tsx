import {
  BarGauge,
  DashboardGrid,
  Disclosure,
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
  useTable,
} from '../artifact.ts'
import { bytes } from '../format.tsx'
import { deltas, points, rates, useSamples, type Values } from '../series.ts'
import { ChartPanel, GapPanel, Note, Panel } from './panels.tsx'

const MIB = 1024 ** 2
const percent = (value: number) => `${value.toFixed(1)}%`
const perSecond = (value: number) => `${value.toFixed(1)}/s`
const mibPerSecond = (value: number) => `${value.toFixed(2)} MiB/s`

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
      sub="client backends, stacked"
      span={8}
      loading={activity.isPending || samples.isPending}
      label="Client backends by state"
      variant="stacked-area"
      min={0}
      formatValue={(value) => value.toFixed(0)}
      data={chart}
      series={series}
      note="Count of pg_stat_activity rows per state, aggregated by the probe"
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
      sub="per interval"
      span={4}
      loading={stats.isPending}
      label="Cache hit ratio (%)"
      max={100}
      formatValue={percent}
      data={chart}
      series={[{ key: 'ratio', label: 'All databases' }]}
      note="Δblks_hit ÷ (Δblks_hit + Δblks_read) between samples"
    />
  )
}

function Transactions() {
  const stats = useDatabaseStats()
  const chart = rates(stats.samples, stats.rows, totals('xact_commit', 'xact_rollback'))
  return (
    <ChartPanel
      title="Transactions"
      sub="commits and rollbacks per second"
      span={6}
      loading={stats.isPending}
      label="Transactions per second"
      min={0}
      formatValue={perSecond}
      data={chart}
      series={[
        { key: 'xact_commit', label: 'Commits' },
        { key: 'xact_rollback', label: 'Rollbacks' },
      ]}
      note="Change in xact_commit and xact_rollback ÷ seconds, all databases"
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
      span={6}
      loading={stats.isPending}
      label="Temporary files written (MiB/s)"
      min={0}
      formatValue={mibPerSecond}
      data={chart}
      series={[{ key: 'temp_bytes', label: 'All databases' }]}
      note="Change in temp_bytes ÷ seconds"
    />
  )
}

function WalAndCheckpoints() {
  const samples = useSamples()
  const target = useTable('postgres_target')
  const bgwriter = useTable('postgres_bgwriter_stats')
  const checkpointer = useTable('postgres_checkpointer_stats')
  const wal = useTable('postgres_wal_stats')
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
  const walRates = rates(sampleList, wal.data ?? [], (rows) => ({
    wal: rows.length ? (num(rows[0], 'wal_bytes') ?? 0) / MIB : null,
  }))
  const hasWal = (wal.data?.length ?? 0) > 0
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
      span={6}
      loading={
        samples.isPending || bgwriter.isPending || checkpointer.isPending || wal.isPending
      }
      label="Bytes written (MiB/s)"
      min={0}
      formatValue={mibPerSecond}
      data={chart}
      series={series}
      note={`Buffers written per second × block size (${blockSize} bytes).${hasWal ? ' WAL from pg_stat_wal.wal_bytes.' : ' pg_stat_wal needs PostgreSQL 14.'}`}
    />
  )
}

function ReplicationLag() {
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
            <TableHead>State</TableHead>
            <TableHead>Sync state</TableHead>
            <TableHead className="num">Standbys</TableHead>
            <TableHead className="num">Max lag</TableHead>
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
    content = <Note>No standby was connected in the last sample.</Note>
  } else {
    const reason = missingReason(outcomes, 'postgres.aggregates.replication')
    content = <Note>Not recorded in this artifact{reason ? `: ${reason}` : ''}.</Note>
  }
  return (
    <Panel title="Replication lag" sub="last sample, from pg_stat_replication" span={6} loading={isPending}>
      {content}
    </Panel>
  )
}

const IEC = ['B', 'KiB', 'MiB', 'GiB']

function DatabaseSize() {
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
    <Panel title="Database size" sub="point in time" span={6} loading={isPending}>
      {sized.length > 0 ? (
        <>
          <BarGauge
            label="Size per database"
            max={largest}
            formatValue={format}
            rows={sized.map((row) => ({
              key: String(row.database_name),
              label: String(row.database_name),
              value: num(row, 'size_bytes'),
            }))}
          />
          <Note>pg_database_size(), read once per snapshot</Note>
        </>
      ) : (
        <Note>Not recorded in this artifact.</Note>
      )}
    </Panel>
  )
}

export function Postgres() {
  return (
    <Disclosure summary="PostgreSQL" open>
      <DashboardGrid>
        <ConnectionsByState />
        <CacheHitRatio />
        <Transactions />
        <TemporaryFiles />
        <WalAndCheckpoints />
        <ReplicationLag />
        <DatabaseSize />
        <GapPanel title="Largest tables" sub="heap, top 6" span={6} gap="no collector">
          Only whole-database size exists. The plan is a new postgres.stats.relations collector.
        </GapPanel>
        <GapPanel title="Dead tuples" sub="vacuum backlog, top 6" span={12} gap="no collector">
          Nothing reads pg_stat_user_tables yet. It comes with postgres.stats.relations.
        </GapPanel>
      </DashboardGrid>
    </Disclosure>
  )
}
