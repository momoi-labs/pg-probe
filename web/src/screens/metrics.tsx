import {
  Card,
  DashboardGrid,
  DashboardPanel,
  Disclosure,
  Meter,
  Skeleton,
  Sparkline,
  Stat,
  StatFoot,
  StatLabel,
  StatValue,
} from '@momoi-labs/kiso-react'
import type { ReactNode } from 'react'
import type { Row } from '../api.ts'
import {
  collected,
  integer,
  latest,
  num,
  sum,
  useOutcomes,
  useTable,
} from '../artifact.ts'
import { bytes, Bytes, Percent, Unknown } from '../format.tsx'
import { deltas, lastValue, points, useSamples } from '../series.ts'
import { isClientBackend, Postgres, useWalRate } from './postgres.tsx'
import { System, useCpuShares, useDiskBusy } from './system.tsx'

const NOT_RECORDED = 'not recorded in this artifact'
const NEEDS_SAMPLES = 'a rate needs two or more samples, and this artifact holds one'

type TileProps = {
  label: string
  loading?: boolean
  value: ReactNode
  meter?: ReactNode
  foot: ReactNode
  // One value per sample. Drawn as a sparkline when there are two or more.
  trend?: (number | null)[]
}

function Tile({ label, loading, value, meter, foot, trend }: TileProps) {
  return (
    <DashboardPanel span={3}>
      <Card>
        <Stat>
          <StatLabel>{label}</StatLabel>
          {loading ? (
            <Skeleton variant="text" />
          ) : (
            <>
              <StatValue>{value}</StatValue>
              {meter}
              <StatFoot>{foot}</StatFoot>
              {trend && trend.length > 1 && (
                <Sparkline values={trend} height={28} label={`${label} over the session`} />
              )}
            </>
          )}
        </Stat>
      </Card>
    </DashboardPanel>
  )
}

function UnknownTile({ label, reason }: { label: string; reason: string }) {
  return <Tile label={label} value={<Unknown reason={reason} />} foot={`Unknown: ${reason}.`} />
}

function CpuTile() {
  const { chart, isPending } = useCpuShares()
  if (isPending) return <Tile label="CPU busy" loading value={null} foot={null} />
  const trend = chart.map((point) => {
    const parts = Object.values(point.values)
    return parts.some((value) => value == null)
      ? null
      : parts.reduce<number>((total, value) => total + (value ?? 0), 0)
  })
  const current = trend.findLast((value) => value != null) ?? null
  if (current == null) return <UnknownTile label="CPU busy" reason={NEEDS_SAMPLES} />
  return (
    <Tile
      label="CPU busy"
      value={<Percent value={current} />}
      foot="user + system + iowait + steal + nice, IRQ and softirq, last interval"
      trend={trend.slice(1)}
    />
  )
}

function MemoryTile() {
  const samples = useSamples()
  const { data, isPending } = useTable('linux_memory')
  if (isPending || samples.isPending) {
    return <Tile label="Memory used" loading value={null} foot={null} />
  }
  const value = (rows: Row[], key: string) =>
    num(
      rows.find((row) => row.key === key),
      'value_bytes',
    )
  const chart = points(samples.data ?? [], data ?? [], (rows) => {
    const total = value(rows, 'MemTotal')
    const available = value(rows, 'MemAvailable')
    return {
      used: total == null || available == null || total === 0 ? null : total - available,
      total,
    }
  })
  const used = lastValue(chart, 'used')
  const total = lastValue(chart, 'total')
  if (used == null || total == null) return <UnknownTile label="Memory used" reason={NOT_RECORDED} />
  const usedText = bytes(used)
  const totalText = bytes(total)
  return (
    <Tile
      label="Memory used"
      value={<Percent value={(used / total) * 100} />}
      meter={
        <Meter
          label="Used of MemTotal"
          value={used}
          max={total}
          valueText={`${usedText.number} ${usedText.unit} of ${totalText.number} ${totalText.unit}`}
        />
      }
      foot="MemTotal − MemAvailable, last sample"
      trend={chart.map((point) => point.values.used ?? null)}
    />
  )
}

function DiskTile() {
  const { all, isPending } = useDiskBusy()
  if (isPending) return <Tile label="Disk busy" loading value={null} foot={null} />
  // The busiest disk in each interval.
  const busiest = all.map((point) => {
    const entries = Object.entries(point.values).filter(
      (entry): entry is [string, number] => entry[1] != null,
    )
    if (entries.length === 0) return null
    return entries.reduce((best, entry) => (entry[1] > best[1] ? entry : best))
  })
  const current = busiest.findLast((entry) => entry != null) ?? null
  if (current == null) return <UnknownTile label="Disk busy" reason={NEEDS_SAMPLES} />
  return (
    <Tile
      label="Disk busy"
      value={<Percent value={current[1]} />}
      foot={`${current[0]}, the busiest disk in the last interval`}
      trend={busiest.slice(1).map((entry) => entry?.[1] ?? null)}
    />
  )
}

function LoadTile() {
  const samples = useSamples()
  const scheduler = useTable('linux_scheduler')
  const cpus = useTable('linux_cpu_set')
  if (scheduler.isPending || cpus.isPending || samples.isPending) {
    return <Tile label="Load, 1 minute" loading value={null} foot={null} />
  }
  const chart = points(samples.data ?? [], scheduler.data ?? [], (rows) => ({
    load: num(rows[0], 'load_avg_1m'),
  }))
  const load = lastValue(chart, 'load')
  const online = cpus.data?.filter((row) => row.set_kind === 'online').length ?? 0
  if (load == null) return <UnknownTile label="Load, 1 minute" reason={NOT_RECORDED} />
  return (
    <Tile
      label="Load, 1 minute"
      value={load.toFixed(2)}
      foot={
        online > 0
          ? `${(load / online).toFixed(2)} per CPU · ${online} CPUs online`
          : 'Online CPU count not recorded'
      }
      trend={chart.map((point) => point.values.load ?? null)}
    />
  )
}

function ConnectionsTile() {
  const samples = useSamples()
  const activity = useTable('postgres_activity_aggregates')
  const settings = useTable('postgres_setting')
  if (activity.isPending || settings.isPending || samples.isPending) {
    return <Tile label="Connections" loading value={null} foot={null} />
  }
  if (!activity.data?.length) return <UnknownTile label="Connections" reason={NOT_RECORDED} />
  const chart = points(samples.data ?? [], activity.data.filter(isClientBackend), (rows) => ({
    connections: rows.length ? sum(rows, 'count') : null,
  }))
  const connections = lastValue(chart, 'connections') ?? 0
  const setting = settings.data?.find((row) => row.setting_name === 'max_connections')
  const max = setting?.setting_value == null ? null : Number(setting.setting_value)
  return (
    <Tile
      label="Connections"
      value={integer(connections)}
      meter={
        max != null && max > 0 ? (
          <Meter
            label="Of max_connections"
            value={connections}
            max={max}
            valueText={`${integer(connections)} of ${integer(max)}`}
          />
        ) : undefined
      }
      foot="Client backends in pg_stat_activity, last sample"
      trend={chart.map((point) => point.values.connections ?? null)}
    />
  )
}

function CacheHitTile() {
  const samples = useSamples()
  const { data, isPending } = useTable('postgres_database_stats')
  if (isPending || samples.isPending) {
    return <Tile label="Cache hit ratio" loading value={null} foot={null} />
  }
  if (!data?.length) return <UnknownTile label="Cache hit ratio" reason={NOT_RECORDED} />
  const chart = deltas(
    samples.data ?? [],
    data,
    (rows) => ({
      hit: rows.length ? sum(rows, 'blks_hit') : null,
      read: rows.length ? sum(rows, 'blks_read') : null,
    }),
    (delta) => ({
      ratio:
        delta.hit == null || delta.read == null || delta.hit + delta.read === 0
          ? null
          : (delta.hit / (delta.hit + delta.read)) * 100,
    }),
  )
  const interval = lastValue(chart, 'ratio')
  if (interval != null) {
    return (
      <Tile
        label="Cache hit ratio"
        value={<Percent value={interval} />}
        foot="Δblks_hit ÷ (Δblks_hit + Δblks_read), all databases, last interval"
        trend={chart.slice(1).map((point) => point.values.ratio ?? null)}
      />
    )
  }
  // One sample: only the ratio since the last statistics reset is known.
  const rows = latest(data)
  const hit = sum(rows, 'blks_hit')
  const read = sum(rows, 'blks_read')
  if (hit + read === 0) return <UnknownTile label="Cache hit ratio" reason={NOT_RECORDED} />
  return (
    <Tile
      label="Cache hit ratio"
      value={<Percent value={(hit / (hit + read)) * 100} />}
      foot="blks_hit ÷ (blks_hit + blks_read), all databases, since the last statistics reset"
    />
  )
}

function WalTile() {
  const { chart, source, isPending } = useWalRate()
  if (isPending) return <Tile label="WAL generated" loading value={null} foot={null} />
  if (source == null) return <UnknownTile label="WAL generated" reason={NOT_RECORDED} />
  const current = lastValue(chart, 'wal')
  if (current == null) return <UnknownTile label="WAL generated" reason={NEEDS_SAMPLES} />
  const perSecond = bytes(current * 1024 ** 2)
  return (
    <Tile
      label="WAL generated"
      value={`${perSecond.number} ${perSecond.unit}/s`}
      foot={`Change in ${source} ÷ seconds, last interval`}
      trend={chart.slice(1).map((point) => point.values.wal ?? null)}
    />
  )
}

function ReplicaLagTile() {
  const outcomes = useOutcomes()
  const replication = useTable('postgres_replication_aggregates')
  if (replication.isPending) return <Tile label="Replica lag" loading value={null} foot={null} />
  const data = latest(replication.data)
  // No rows means no standby only when the collector ran.
  if (!data.length && !collected(outcomes, 'postgres.aggregates.replication')) {
    return <UnknownTile label="Replica lag" reason={NOT_RECORDED} />
  }
  if (!data.length) {
    return (
      <Tile
        label="Replica lag"
        value={<span className="muted">None</span>}
        foot="No standby was connected in the last sample"
      />
    )
  }
  const lags = data.map((row) => num(row, 'max_lag_bytes')).filter((lag) => lag != null)
  if (lags.length === 0) return <UnknownTile label="Replica lag" reason={NOT_RECORDED} />
  return (
    <Tile
      label="Replica lag"
      value={<Bytes value={Math.max(...lags)} />}
      foot="Largest max_lag_bytes across standbys, last sample"
    />
  )
}

export function Metrics() {
  return (
    <>
      <Overview />
      <System />
      <Postgres />
    </>
  )
}

function Overview() {
  return (
    <Disclosure summary="Overview" open>
      <DashboardGrid>
        <CpuTile />
        <MemoryTile />
        <DiskTile />
        <LoadTile />
        <ConnectionsTile />
        <CacheHitTile />
        <WalTile />
        <ReplicaLagTile />
      </DashboardGrid>
    </Disclosure>
  )
}
