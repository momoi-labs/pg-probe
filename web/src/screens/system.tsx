import { DashboardGrid, Disclosure, Meter } from '@momoi-labs/kiso-react'
import type { Row } from '../api.ts'
import { missingReason, num, useOutcomes, useTable } from '../artifact.ts'
import { deltas, points, rates, useSamples, type Values } from '../series.ts'
import { ChartPanel, Note, Panel } from './panels.tsx'

const GIB = 1024 ** 3
const MIB = 1024 ** 2
const oneDecimal = (value: number) => value.toFixed(1)
const gib = (value: number) => `${value.toFixed(1)} GiB`
const decimal = (value: number) => value.toFixed(2)

// Kernel sector unit for /proc/diskstats, not the device block size.
const SECTOR_BYTES = 512

const memoryValue = (rows: Row[], key: string) =>
  num(
    rows.find((row) => row.key === key),
    'value_bytes',
  )

// Guest time is already counted in user time, so it stays out of the total.
const CPU_STATES = ['user', 'nice', 'system', 'idle', 'iowait', 'irq', 'softirq', 'steal']

function cpuTicks(rows: Row[]): Values {
  const ticks = (state: string) => num(rows.find((row) => row.state === state), 'ticks')
  const values: Values = {}
  for (const state of CPU_STATES) values[state] = ticks(state)
  return values
}

// Share of CPU time per state between samples, for the chart and the
// Overview tile.
export function useCpuShares() {
  const samples = useSamples()
  const { data, isPending } = useTable('linux_cpu_time', { cpu_id: '-1' })
  const chart = deltas(samples.data ?? [], data ?? [], cpuTicks, (delta) => {
    const known = CPU_STATES.map((state) => delta[state])
    if (known.some((value) => value == null)) {
      return { user: null, system: null, iowait: null, steal: null, other: null }
    }
    const total = known.reduce((a, b) => (a ?? 0) + (b ?? 0), 0) ?? 0
    const share = (value: number) => (total > 0 ? (value / total) * 100 : null)
    return {
      user: share(delta.user ?? 0),
      system: share(delta.system ?? 0),
      iowait: share(delta.iowait ?? 0),
      steal: share(delta.steal ?? 0),
      other: share((delta.nice ?? 0) + (delta.irq ?? 0) + (delta.softirq ?? 0)),
    }
  })
  return { chart, isPending: isPending || samples.isPending }
}

function CpuUtilization() {
  const { chart, isPending } = useCpuShares()
  return (
    <ChartPanel
      title="CPU utilization"
      span={8}
      loading={isPending}
      label="CPU utilization by state (%)"
      variant="stacked-area"
      min={0}
      max={100}
      formatValue={oneDecimal}
      data={chart}
      series={[
        { key: 'user', label: 'User' },
        { key: 'system', label: 'System' },
        { key: 'iowait', label: 'I/O wait' },
        { key: 'steal', label: 'Steal' },
        { key: 'other', label: 'Nice, IRQ, softirq' },
      ]}
      note="Change in /proc/stat ticks between samples ÷ change in all ticks. The mean across CPUs, not the sum."
    />
  )
}

function LoadAverage() {
  const samples = useSamples()
  const { data, isPending } = useTable('linux_scheduler')
  const chart = points(samples.data ?? [], data ?? [], (rows) => ({
    '1m': num(rows[0], 'load_avg_1m'),
    '5m': num(rows[0], 'load_avg_5m'),
    '15m': num(rows[0], 'load_avg_15m'),
  }))
  return (
    <ChartPanel
      title="Load average"
      span={4}
      loading={isPending || samples.isPending}
      label="Load average, 1 / 5 / 15 minutes"
      min={0}
      formatValue={decimal}
      data={chart}
      series={[
        { key: '1m', label: '1 minute' },
        { key: '5m', label: '5 minutes' },
        { key: '15m', label: '15 minutes' },
      ]}
    />
  )
}

function Memory() {
  const samples = useSamples()
  const { data, isPending } = useTable('linux_memory')
  let total: number | null = null
  const chart = points(samples.data ?? [], data ?? [], (rows) => {
    const value = (key: string) => memoryValue(rows, key)
    const memTotal = value('MemTotal')
    const free = value('MemFree')
    const buffers = value('Buffers')
    const cached = value('Cached')
    const shmem = value('Shmem')
    const reclaimable = value('SReclaimable')
    if (
      memTotal == null ||
      free == null ||
      buffers == null ||
      cached == null ||
      shmem == null ||
      reclaimable == null
    ) {
      return { used: null, shmem: null, cache: null, buffers: null, free: null }
    }
    total = memTotal
    return {
      used: (memTotal - free - buffers - cached - reclaimable) / GIB,
      shmem: shmem / GIB,
      cache: (cached - shmem + reclaimable) / GIB,
      buffers: buffers / GIB,
      free: free / GIB,
    }
  })
  const max = total == null ? undefined : (total as number) / GIB
  return (
    <ChartPanel
      title="Memory"
      span={8}
      loading={isPending || samples.isPending}
      label="Memory by use, up to MemTotal (GiB)"
      variant="stacked-area"
      min={0}
      max={max}
      formatValue={oneDecimal}
      data={chart}
      series={[
        { key: 'used', label: 'Used' },
        { key: 'shmem', label: 'Shared memory' },
        { key: 'cache', label: 'Page cache' },
        { key: 'buffers', label: 'Buffers' },
        { key: 'free', label: 'Free' },
      ]}
      note="From /proc/meminfo. Page cache is Cached − Shmem + SReclaimable, because Cached already includes Shmem. Used is MemTotal − MemFree − Buffers − Cached − SReclaimable. Shared memory includes shared_buffers."
    />
  )
}

function Swap() {
  const samples = useSamples()
  const { data, isPending } = useTable('linux_memory')
  const chart = points(samples.data ?? [], data ?? [], (rows) => {
    const total = memoryValue(rows, 'SwapTotal')
    const free = memoryValue(rows, 'SwapFree')
    return { used: total == null || free == null ? null : (total - free) / GIB }
  })
  return (
    <ChartPanel
      title="Swap in use"
      span={4}
      loading={isPending || samples.isPending}
      label="Swap used (GiB)"
      min={0}
      formatValue={oneDecimal}
      data={chart}
      series={[{ key: 'used', label: 'Used' }]}
      note="SwapTotal − SwapFree"
    />
  )
}

// Whole disks only. Partitions repeat their disk's I/O and would count twice.
function useDisks() {
  const devices = useTable('linux_block_device')
  const io = useTable('linux_block_io')
  const names = new Map<string, string>()
  for (const row of devices.data ?? []) {
    names.set(`${row.device_major}:${row.device_minor}`, String(row.kernel_name))
  }
  const rows = (io.data ?? []).flatMap((row) => {
    const name = names.get(`${row.device_major}:${row.device_minor}`)
    return name ? [{ ...row, kernel_name: name }] : []
  })
  return { rows, names: [...new Set(names.values())], isPending: devices.isPending || io.isPending }
}

function DiskThroughput() {
  const samples = useSamples()
  const disks = useDisks()
  const chart = rates(samples.data ?? [], disks.rows, (rows) => {
    const sectors = (column: string) =>
      rows.length === 0 ? null : rows.reduce((total, row) => total + (num(row, column) ?? 0), 0)
    const read = sectors('sectors_read')
    const written = sectors('sectors_written')
    return {
      read: read == null ? null : (read * SECTOR_BYTES) / MIB,
      write: written == null ? null : (written * SECTOR_BYTES) / MIB,
    }
  })
  return (
    <ChartPanel
      title="Disk throughput"
      span={6}
      loading={disks.isPending || samples.isPending}
      label="Disk throughput, all disks (MiB/s)"
      min={0}
      formatValue={oneDecimal}
      data={chart}
      series={[
        { key: 'read', label: 'Read' },
        { key: 'write', label: 'Write' },
      ]}
      note="Change in sectors read and written × 512 bytes ÷ seconds between samples, summed over whole disks."
    />
  )
}

// Busy share of every whole disk between samples, for the chart and the
// Overview tile.
export function useDiskBusy() {
  const samples = useSamples()
  const disks = useDisks()
  const all = deltas(
    samples.data ?? [],
    disks.rows,
    (rows) =>
      Object.fromEntries(
        disks.names.map((name) => [
          name,
          num(
            rows.find((row) => row.kernel_name === name),
            'io_milliseconds',
          ),
        ]),
      ),
    (delta, seconds) =>
      Object.fromEntries(
        Object.entries(delta).map(([name, ms]) => [
          name,
          ms == null ? null : Math.min((ms / (seconds * 1000)) * 100, 100),
        ]),
      ),
  )
  return { all, names: disks.names, isPending: disks.isPending || samples.isPending }
}

function DiskUtilization() {
  const { all, names, isPending } = useDiskBusy()
  // A chart holds five series: keep the five busiest disks.
  const busiest = [...names]
    .map((name) => ({
      name,
      total: all.reduce((sum, sample) => sum + (sample.values[name] ?? 0), 0),
    }))
    .sort((a, b) => b.total - a.total)
    .slice(0, 5)
    .map(({ name }) => name)
  const chart = all.map((sample) => ({
    timestamp: sample.timestamp,
    values: Object.fromEntries(busiest.map((name) => [name, sample.values[name] ?? null])),
  }))
  return (
    <ChartPanel
      title="Disk utilization"
      span={6}
      loading={isPending}
      label="Disk busy time, five busiest disks (%)"
      min={0}
      max={100}
      formatValue={oneDecimal}
      data={chart}
      series={busiest.map((name) => ({ key: name, label: name }))}
      note="Change in io_milliseconds ÷ milliseconds between samples. On SSDs and NVMe with deep queues this nears 100% before the device is saturated."
    />
  )
}

function PressureStall() {
  const samples = useSamples()
  const { data, isPending } = useTable('linux_psi', { scope: 'some' })
  const outcomes = useOutcomes()
  if (!isPending && !data?.length) {
    const reason = missingReason(outcomes, 'proc.pressure')
    return (
      <Panel title="Pressure stall" sub="some, 10s window" span={6}>
        <Note>
          Not recorded in this artifact
          {reason
            ? `: proc.pressure is ${reason}. Older kernels and some containers have no PSI.`
            : '.'}
        </Note>
      </Panel>
    )
  }
  const chart = points(samples.data ?? [], data ?? [], (rows) => ({
    cpu: num(
      rows.find((row) => row.resource === 'cpu'),
      'avg10',
    ),
    memory: num(
      rows.find((row) => row.resource === 'memory'),
      'avg10',
    ),
    io: num(
      rows.find((row) => row.resource === 'io'),
      'avg10',
    ),
  }))
  return (
    <ChartPanel
      title="Pressure stall"
      span={6}
      loading={isPending || samples.isPending}
      label="Pressure stall, some tasks, 10 s average (%)"
      min={0}
      formatValue={oneDecimal}
      data={chart}
      series={[
        { key: 'cpu', label: 'CPU' },
        { key: 'memory', label: 'Memory' },
        { key: 'io', label: 'I/O' },
      ]}
      note="avg10 from /proc/pressure"
    />
  )
}

function FilesystemUsage() {
  const { data, isPending } = useTable('linux_mount')
  const mounts = (data ?? [])
    .filter((row) => row.mount_point != null && num(row, 'total_bytes') != null)
    .sort((a, b) => (num(b, 'total_bytes') ?? 0) - (num(a, 'total_bytes') ?? 0))
  return (
    <Panel title="Filesystem usage" sub="point in time, local disks" span={6} loading={isPending}>
      {mounts.length > 0 ? (
        <>
          <div className="stack-sm">
            {mounts.map((row) => {
              const total = num(row, 'total_bytes') ?? 0
              const used = total - (num(row, 'available_bytes') ?? 0)
              return (
                <Meter
                  key={String(row.mount_point)}
                  label={`${row.mount_point} (${row.filesystem_type})`}
                  value={used}
                  max={total}
                  valueText={`${gib(used / GIB)} of ${gib(total / GIB)}`}
                />
              )
            })}
          </div>
          <Note>
            statvfs on local disk filesystems, read once. Network filesystems and /home are not
            recorded. Used counts space reserved for root.
          </Note>
        </>
      ) : (
        <Note>
          Not recorded in this artifact. Captures before schema version 3 have no mount sizes.
        </Note>
      )}
    </Panel>
  )
}

function NetworkThroughput() {
  const samples = useSamples()
  const { data, isPending } = useTable('linux_network_io')
  const interfaces = [...new Set((data ?? []).map((row) => String(row.interface)))]
  if (!isPending && interfaces.length === 0) {
    return (
      <Panel title="Network throughput" sub="all interfaces" span={6}>
        <Note>
          Not recorded in this artifact. Captures before schema version 3 have no network
          counters.
        </Note>
      </Panel>
    )
  }
  const chart = rates(samples.data ?? [], data ?? [], (rows) =>
    rows.length
      ? {
          received: rows.reduce((total, row) => total + (num(row, 'rx_bytes') ?? 0), 0) / MIB,
          sent: rows.reduce((total, row) => total + (num(row, 'tx_bytes') ?? 0), 0) / MIB,
        }
      : { received: null, sent: null },
  )
  return (
    <ChartPanel
      title="Network throughput"
      span={6}
      loading={isPending || samples.isPending}
      label="Network throughput, all interfaces (MiB/s)"
      min={0}
      formatValue={oneDecimal}
      data={chart}
      series={[
        { key: 'received', label: 'Received' },
        { key: 'sent', label: 'Sent' },
      ]}
      note={`Change in /proc/net/dev bytes ÷ seconds, summed over ${interfaces.join(', ')}. Loopback and container links are not recorded.`}
    />
  )
}

function Processes() {
  const samples = useSamples()
  const { data, isPending } = useTable('linux_scheduler')
  const chart = points(samples.data ?? [], data ?? [], (rows) => ({
    running: num(rows[0], 'procs_running'),
    blocked: num(rows[0], 'procs_blocked'),
  }))
  return (
    <ChartPanel
      title="Processes"
      span={6}
      loading={isPending || samples.isPending}
      label="Processes running and blocked"
      min={0}
      formatValue={(value) => value.toFixed(0)}
      data={chart}
      series={[
        { key: 'running', label: 'Running' },
        { key: 'blocked', label: 'Blocked on I/O' },
      ]}
      note="procs_running and procs_blocked from /proc/stat"
    />
  )
}

export function System() {
  return (
    <Disclosure summary="System" open>
      <DashboardGrid>
        <CpuUtilization />
        <LoadAverage />
        <Memory />
        <Swap />
        <DiskThroughput />
        <DiskUtilization />
        <NetworkThroughput />
        <PressureStall />
        <FilesystemUsage />
        <Processes />
      </DashboardGrid>
    </Disclosure>
  )
}
