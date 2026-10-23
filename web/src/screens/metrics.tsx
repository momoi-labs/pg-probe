import {
  Card,
  Skeleton,
  Stat,
  StatFoot,
  StatLabel,
  StatValue,
  type ChartSample,
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
import { useT, type Translate } from '../i18n.ts'
import { useInReport } from '../report.ts'
import { Note, panelId, Section } from './panels.tsx'
import { isClientBackend, Postgres, useWalRate } from './postgres.tsx'
import { System, useCpuShares, useDiskBusy } from './system.tsx'

const NOT_RECORDED = 'not recorded in this artifact'
const NEEDS_SAMPLES = 'a rate needs two or more samples, and this artifact holds one'

// A session series behind a tile: one value per sample or interval, with its
// time. The tile names its highest point, or its lowest when lower is the
// notable direction (cache hit ratio), so the incident is not lost behind a
// calm last value.
type Peak = {
  values: (number | null)[]
  at: number[]
  format: (value: number) => string
  lowest?: boolean
}

type TileProps = {
  label: string
  loading?: boolean
  value: ReactNode
  context?: ReactNode
  peak?: Peak
  // Title of the panel with the full chart. The tile links to it.
  chart?: string
}

function peakText(peak: Peak, t: Translate) {
  let best: number | null = null
  let when = 0
  peak.values.forEach((value, index) => {
    if (value == null) return
    if (best == null || (peak.lowest ? value < best : value > best)) {
      best = value
      when = peak.at[index]
    }
  })
  if (best == null || peak.values.filter((value) => value != null).length < 2) return null
  const time = new Date(when).toISOString().slice(11, 16)
  const values = { value: peak.format(best), time }
  return peak.lowest ? t('Lowest {value} at {time}', values) : t('Peak {value} at {time}', values)
}

// The label is translated here. Callers translate the context.
function Tile({ label, loading, value, context, peak, chart }: TileProps) {
  const t = useT()
  const peakLine = peak && peakText(peak, t)
  const card = (
    <Card>
      <Stat>
        <StatLabel>{t(label)}</StatLabel>
        {loading ? (
          <Skeleton variant="text" />
        ) : (
          <>
            <StatValue>{value}</StatValue>
            {context && <StatFoot>{context}</StatFoot>}
            {peakLine && <StatFoot>{peakLine}</StatFoot>}
          </>
        )}
      </Stat>
    </Card>
  )
  return (
    <div>
      {chart ? (
        <a className="tile-link" href={`#${panelId(chart)}`} aria-label={t('{label}: open the {chart} chart', { label: t(label), chart: t(chart) })}>
          {card}
        </a>
      ) : (
        card
      )}
    </div>
  )
}

const pct = (value: number) => `${value.toFixed(1)}%`
const times = (data: ChartSample[]) => data.map((point) => point.timestamp)

function UnknownTile({ label, reason }: { label: string; reason: string }) {
  const t = useT()
  return (
    <Tile
      label={label}
      value={<Unknown reason={t(reason)} />}
      context={t('Unknown: {reason}.', { reason: t(reason) })}
    />
  )
}

function CpuTile() {
  const { chart, isPending } = useCpuShares()
  if (isPending) return <Tile label="CPU busy" loading value={null}  />
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
      peak={{ values: trend, at: times(chart), format: pct }}
      chart="CPU utilization"
    />
  )
}

function MemoryTile() {
  const t = useT()
  const samples = useSamples()
  const { data, isPending } = useTable('linux_memory')
  if (isPending || samples.isPending) {
    return <Tile label="Memory used" loading value={null}  />
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
      context={t('{used} of {total} {unit}', { used: usedText.number, total: totalText.number, unit: totalText.unit })}
      peak={{
        values: chart.map((point) => {
          const used = point.values.used
          const memTotal = point.values.total
          return used == null || memTotal == null ? null : (used / memTotal) * 100
        }),
        at: times(chart),
        format: pct,
      }}
      chart="Memory"
    />
  )
}

function DiskTile() {
  const t = useT()
  const { all, isPending } = useDiskBusy()
  if (isPending) return <Tile label="Disk busy" loading value={null}  />
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
      context={t('{disk}, the busiest disk', { disk: current[0] })}
      peak={{ values: busiest.map((entry) => entry?.[1] ?? null), at: times(all), format: pct }}
      chart="Disk utilization"
    />
  )
}

function LoadTile() {
  const t = useT()
  const samples = useSamples()
  const scheduler = useTable('linux_scheduler')
  const cpus = useTable('linux_cpu_set')
  if (scheduler.isPending || cpus.isPending || samples.isPending) {
    return <Tile label="Load, 1 minute" loading value={null}  />
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
      context={online > 0 ? t('{count} CPUs online', { count: online }) : undefined}
      peak={{
        values: chart.map((point) => point.values.load ?? null),
        at: times(chart),
        format: (value) => value.toFixed(2),
      }}
      chart="Load average"
    />
  )
}

function ConnectionsTile() {
  const t = useT()
  const samples = useSamples()
  const activity = useTable('postgres_activity_aggregates')
  const settings = useTable('postgres_setting')
  if (activity.isPending || settings.isPending || samples.isPending) {
    return <Tile label="Connections" loading value={null}  />
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
      context={max != null && max > 0 ? t('of {max} max_connections', { max: integer(max) }) : undefined}
      peak={{
        values: chart.map((point) => point.values.connections ?? null),
        at: times(chart),
        format: integer,
      }}
      chart="Connections by state"
    />
  )
}

function CacheHitTile() {
  const t = useT()
  const samples = useSamples()
  const { data, isPending } = useTable('postgres_database_stats')
  if (isPending || samples.isPending) {
    return <Tile label="Cache hit ratio" loading value={null}  />
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
        peak={{
          values: chart.map((point) => point.values.ratio ?? null),
          at: times(chart),
          format: pct,
          lowest: true,
        }}
        chart="Cache hit ratio"
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
      context={t('since the last statistics reset')}
    />
  )
}

function WalTile() {
  const { chart, source, isPending } = useWalRate()
  if (isPending) return <Tile label="WAL generated" loading value={null}  />
  if (source == null) return <UnknownTile label="WAL generated" reason={NOT_RECORDED} />
  const current = lastValue(chart, 'wal')
  if (current == null) return <UnknownTile label="WAL generated" reason={NEEDS_SAMPLES} />
  const perSecond = bytes(current * 1024 ** 2)
  return (
    <Tile
      label="WAL generated"
      value={`${perSecond.number} ${perSecond.unit}/s`}
      peak={{
        values: chart.map((point) => point.values.wal ?? null),
        at: times(chart),
        format: (value) => {
          const rate = bytes(value * 1024 ** 2)
          return `${rate.number} ${rate.unit}/s`
        },
      }}
      chart="WAL and checkpoint writes"
    />
  )
}

function ReplicaLagTile() {
  const t = useT()
  const outcomes = useOutcomes()
  const replication = useTable('postgres_replication_aggregates')
  if (replication.isPending) return <Tile label="Replica lag" loading value={null}  />
  const data = latest(replication.data)
  // No rows means no standby only when the collector ran.
  if (!data.length && !collected(outcomes, 'postgres.aggregates.replication')) {
    return <UnknownTile label="Replica lag" reason={NOT_RECORDED} />
  }
  if (!data.length) {
    return (
      <Tile
        label="Replica lag"
        value={<span className="muted">{t('None')}</span>}
        context={t('No standby connected')}
      />
    )
  }
  const lags = data.map((row) => num(row, 'max_lag_bytes')).filter((lag) => lag != null)
  if (lags.length === 0) return <UnknownTile label="Replica lag" reason={NOT_RECORDED} />
  return (
    <Tile
      label="Replica lag"
      value={<Bytes value={Math.max(...lags)} />}
      context={t('Worst standby')}
      chart="Replication lag"
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
  const inReport = useInReport()
  const t = useT()
  return (
    <Section title="Overview">
      <div className="stack-sm">
        <Note>
          {t(
            'Each value is the last sample, or the last interval for rates. Peaks cover the whole session.',
          )}
          {!inReport && ' Select a tile to open its chart.'}
        </Note>
        <div className="tile-grid">
          <CpuTile />
          <MemoryTile />
          <DiskTile />
          <LoadTile />
          <ConnectionsTile />
          <CacheHitTile />
          <WalTile />
          <ReplicaLagTile />
        </div>
      </div>
    </Section>
  )
}
