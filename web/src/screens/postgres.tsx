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
import { ChartPanel, Note, Panel } from './panels.tsx'

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
      span={8}
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
      span={4}
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
      span={6}
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
      span={6}
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
      span={6}
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
        </>
      ) : (
        <Note>Not recorded in this artifact.</Note>
      )}
    </Panel>
  )
}

const relationName = (row: Row) => `${row.database_name}.${row.schema_name}.${row.relation_name}`

function LargestTables() {
  const target = useTable('postgres_target')
  const { data, isPending } = useTable('postgres_relation')
  const blockSize = num(target.data?.[0], 'block_size_bytes') ?? 8192
  const pages = (row: Row) =>
    (num(row, 'heap_pages') ?? 0) + (num(row, 'index_pages') ?? 0) + (num(row, 'toast_pages') ?? 0)
  const largest = [...(data ?? [])].sort((a, b) => pages(b) - pages(a)).slice(0, 6)
  const top = largest.length ? pages(largest[0]) * blockSize : 0
  return (
    <Panel title="Largest tables" sub="heap, indexes and TOAST, top 6" span={6} loading={isPending}>
      {largest.length > 0 ? (
        <>
          <BarGauge
            label="Size per table (GiB)"
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
        <Note>
          Not recorded in this artifact. Captures before schema version 3 have no relations.
        </Note>
      )}
    </Panel>
  )
}

function DeadTuples() {
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
              <TableHead>Table</TableHead>
              <TableHead className="num">Dead tuples</TableHead>
              <TableHead className="num">Live tuples</TableHead>
              <TableHead className="num">Dead (%)</TableHead>
              <TableHead>Last autovacuum</TableHead>
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
    content = <Note>No dead tuples in the recorded tables.</Note>
  } else {
    content = (
      <Note>Not recorded in this artifact. Captures before schema version 3 have no relations.</Note>
    )
  }
  return (
    <Panel title="Dead tuples" sub="vacuum backlog, top 6" span={12} loading={isPending}>
      {content}
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
        <LargestTables />
        <DeadTuples />
      </DashboardGrid>
    </Disclosure>
  )
}
