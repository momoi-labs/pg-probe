import {
  Card,
  DashboardGrid,
  DashboardPanel,
  Disclosure,
  Meter,
  Skeleton,
  Stat,
  StatFoot,
  StatLabel,
  StatValue,
} from '@momoi-labs/kiso-react'
import type { ReactNode } from 'react'
import { collected, integer, num, sum, useOutcomes, useTable } from '../artifact.ts'
import { bytes, Bytes, Percent, Unknown } from '../format.tsx'
import { isClientBackend, Postgres } from './postgres.tsx'
import { System } from './system.tsx'

const NOT_RECORDED = 'not recorded in this artifact'
const NEEDS_SAMPLES = 'a rate needs two or more samples, and artifacts hold one today'

type TileProps = {
  label: string
  loading?: boolean
  value: ReactNode
  meter?: ReactNode
  foot: ReactNode
}

function Tile({ label, loading, value, meter, foot }: TileProps) {
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

function MemoryTile() {
  const { data, isPending } = useTable('linux_memory')
  const value = (key: string) => num(data?.find((row) => row.key === key), 'value_bytes')
  const total = value('MemTotal')
  const available = value('MemAvailable')
  if (isPending) return <Tile label="Memory used" loading value={null} foot={null} />
  if (total == null || available == null || total === 0) {
    return <UnknownTile label="Memory used" reason={NOT_RECORDED} />
  }
  const used = total - available
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
      foot="MemTotal − MemAvailable, from /proc/meminfo"
    />
  )
}

function LoadTile() {
  const scheduler = useTable('linux_scheduler')
  const cpus = useTable('linux_cpu_set')
  const load = num(scheduler.data?.[0], 'load_avg_1m')
  const online = cpus.data?.filter((row) => row.set_kind === 'online').length ?? 0
  if (scheduler.isPending || cpus.isPending) {
    return <Tile label="Load, 1 minute" loading value={null} foot={null} />
  }
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
    />
  )
}

function ConnectionsTile() {
  const activity = useTable('postgres_activity_aggregates')
  const settings = useTable('postgres_setting')
  if (activity.isPending || settings.isPending) {
    return <Tile label="Connections" loading value={null} foot={null} />
  }
  if (!activity.data?.length) return <UnknownTile label="Connections" reason={NOT_RECORDED} />
  const connections = sum(activity.data.filter(isClientBackend), 'count')
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
      foot="Client backends in pg_stat_activity"
    />
  )
}

function CacheHitTile() {
  const { data, isPending } = useTable('postgres_database_stats')
  if (isPending) return <Tile label="Cache hit ratio" loading value={null} foot={null} />
  const hit = sum(data ?? [], 'blks_hit')
  const read = sum(data ?? [], 'blks_read')
  if (!data?.length || hit + read === 0) {
    return <UnknownTile label="Cache hit ratio" reason={NOT_RECORDED} />
  }
  return (
    <Tile
      label="Cache hit ratio"
      value={<Percent value={(hit / (hit + read)) * 100} />}
      foot="blks_hit ÷ (blks_hit + blks_read), all databases, since the last statistics reset"
    />
  )
}

function ReplicaLagTile() {
  const outcomes = useOutcomes()
  const { data, isPending } = useTable('postgres_replication_aggregates')
  if (isPending) return <Tile label="Replica lag" loading value={null} foot={null} />
  // No rows means no standby only when the collector ran.
  if (!data?.length && !collected(outcomes, 'postgres.aggregates.replication')) {
    return <UnknownTile label="Replica lag" reason={NOT_RECORDED} />
  }
  if (!data?.length) {
    return (
      <Tile
        label="Replica lag"
        value={<span className="muted">None</span>}
        foot="No standby was connected when the snapshot was taken"
      />
    )
  }
  const lags = data.map((row) => num(row, 'max_lag_bytes')).filter((lag) => lag != null)
  if (lags.length === 0) return <UnknownTile label="Replica lag" reason={NOT_RECORDED} />
  return (
    <Tile
      label="Replica lag"
      value={<Bytes value={Math.max(...lags)} />}
      foot="Largest max_lag_bytes across standbys"
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
        <UnknownTile label="CPU busy" reason={NEEDS_SAMPLES} />
        <MemoryTile />
        <UnknownTile label="Disk busy" reason={NEEDS_SAMPLES} />
        <LoadTile />
        <ConnectionsTile />
        <CacheHitTile />
        <UnknownTile label="WAL generated" reason={NEEDS_SAMPLES} />
        <ReplicaLagTile />
      </DashboardGrid>
    </Disclosure>
  )
}
