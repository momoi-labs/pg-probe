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
import { collected, integer, missingReason, num, useOutcomes, useTable } from '../artifact.ts'
import { bytes } from '../format.tsx'
import { GapPanel, Note, Panel } from './panels.tsx'

// Rows from servers older than PostgreSQL 10 have no backend type. Every
// pg_stat_activity row on those servers is a client backend.
export const isClientBackend = (row: Row) =>
  row.backend_type == null ||
  row.backend_type === 'client backend' ||
  row.backend_type === '<invalid>'

function ConnectionsByState() {
  const activity = useTable('postgres_activity_aggregates')
  const settings = useTable('postgres_setting')
  const byState = new Map<string, number>()
  for (const row of activity.data?.filter(isClientBackend) ?? []) {
    const state = row.state == null ? 'NULL' : String(row.state)
    byState.set(state, (byState.get(state) ?? 0) + (num(row, 'count') ?? 0))
  }
  const total = [...byState.values()].reduce((a, b) => a + b, 0)
  const setting = settings.data?.find((row) => row.setting_name === 'max_connections')
  const max = setting?.setting_value == null ? null : Number(setting.setting_value)
  return (
    <Panel
      title="Connections by state"
      sub="client backends"
      span={8}
      loading={activity.isPending || settings.isPending}
    >
      {byState.size > 0 ? (
        <>
          <BarGauge
            label={max ? `Client backends by state, of max_connections ${max}` : 'Client backends by state'}
            max={max && max >= total ? max : total}
            formatValue={integer}
            rows={[...byState]
              .sort((a, b) => b[1] - a[1])
              .map(([state, count]) => ({ key: state, label: state, value: count }))}
          />
          <Note>Count of pg_stat_activity rows per state, aggregated by the probe</Note>
        </>
      ) : (
        <Note>Not recorded in this artifact.</Note>
      )}
    </Panel>
  )
}

function ReplicationLag() {
  const { data, isPending } = useTable('postgres_replication_aggregates')
  const outcomes = useOutcomes()
  let content
  if (data?.length) {
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
    content = <Note>No standby was connected when the snapshot was taken.</Note>
  } else {
    const reason = missingReason(outcomes, 'postgres.aggregates.replication')
    content = <Note>Not recorded in this artifact{reason ? `: ${reason}` : ''}.</Note>
  }
  return (
    <Panel title="Replication lag" sub="from pg_stat_replication" span={6} loading={isPending}>
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

const NEEDS_SAMPLING = 'needs sampling'

export function Postgres() {
  return (
    <Disclosure summary="PostgreSQL" open>
      <DashboardGrid>
        <ConnectionsByState />
        <GapPanel title="Cache hit ratio" sub="per interval" span={4} gap={NEEDS_SAMPLING}>
          postgres_database_stats has blks_hit and blks_read. The ratio per interval needs two
          samples. The Overview shows the ratio since the last statistics reset.
        </GapPanel>
        <GapPanel
          title="Transactions"
          sub="commits and rollbacks per second"
          span={6}
          gap={NEEDS_SAMPLING}
        >
          postgres_database_stats has xact_commit and xact_rollback. A rate needs two samples.
        </GapPanel>
        <GapPanel title="Temporary files written" span={6} gap={NEEDS_SAMPLING}>
          postgres_database_stats has temp_files and temp_bytes. A rate needs two samples.
        </GapPanel>
        <GapPanel title="WAL and checkpoint writes" span={6} gap={NEEDS_SAMPLING}>
          postgres_wal_stats and the checkpointer or bgwriter stats have the counters. A rate
          needs two samples.
        </GapPanel>
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
