import { Meter } from '@momoi-labs/kiso-react'
import type { Row } from '../api.ts'
import { missingReason, num, unsupportedCode, useOutcomes, useTable } from '../artifact.ts'
import { useT } from '../i18n.ts'
import { deltas, points, rates, useSamples, type Values } from '../series.ts'
import { ChartPanel, Note, NotSupportedPanel, Panel, PanelGrid, Section, Slot } from './panels.tsx'

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
      loading={isPending || samples.isPending}
      label="Swap used (GiB)"
      min={0}
      formatValue={oneDecimal}
      data={chart}
      series={[{ key: 'used', label: 'Used' }]}
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
      loading={disks.isPending || samples.isPending}
      label="Disk throughput, all disks (MiB/s)"
      min={0}
      formatValue={oneDecimal}
      data={chart}
      series={[
        { key: 'read', label: 'Read' },
        { key: 'write', label: 'Write' },
      ]}
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
      loading={isPending}
      label="Disk busy time, five busiest disks (%)"
      min={0}
      max={100}
      formatValue={oneDecimal}
      data={chart}
      series={busiest.map((name) => ({ key: name, label: name }))}
    />
  )
}

function PressureStall() {
  const t = useT()
  const samples = useSamples()
  const { data, isPending } = useTable('linux_psi', { scope: 'some' })
  const outcomes = useOutcomes()
  if (!isPending && !data?.length) {
    const reason = missingReason(outcomes, 'proc.pressure')
    return (
      <Panel title="Pressure stall" sub="some, 10s window">
        <Note>
          {reason
            ? t(
                'Not recorded in this artifact: proc.pressure is {reason}. Older kernels and some containers have no PSI.',
                { reason },
              )
            : t('Not recorded in this artifact.')}
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
    />
  )
}

function FilesystemUsage() {
  const t = useT()
  const { data, isPending } = useTable('linux_mount')
  const mounts = (data ?? [])
    .filter((row) => row.mount_point != null && num(row, 'total_bytes') != null)
    .sort((a, b) => (num(b, 'total_bytes') ?? 0) - (num(a, 'total_bytes') ?? 0))
  return (
    <Panel title="Filesystem usage" sub="point in time, local disks" loading={isPending}>
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
                  valueText={t('{used} of {total}', { used: gib(used / GIB), total: gib(total / GIB) })}
                  labels={{ notCollected: t('Not collected') }}
                />
              )
            })}
          </div>
        </>
      ) : (
        <Note>{t('Not recorded in this artifact. Captures before schema version 3 have no mount sizes.')}</Note>
      )}
    </Panel>
  )
}

function NetworkThroughput() {
  const t = useT()
  const samples = useSamples()
  const { data, isPending } = useTable('linux_network_io')
  const interfaces = [...new Set((data ?? []).map((row) => String(row.interface)))]
  if (!isPending && interfaces.length === 0) {
    return (
      <Panel title="Network throughput" sub="all interfaces">
        <Note>{t('Not recorded in this artifact. Captures before schema version 3 have no network counters.')}</Note>
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
      loading={isPending || samples.isPending}
      label="Network throughput, all interfaces (MiB/s)"
      min={0}
      formatValue={oneDecimal}
      data={chart}
      series={[
        { key: 'received', label: 'Received' },
        { key: 'sent', label: 'Sent' },
      ]}
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
      loading={isPending || samples.isPending}
      label="Processes running and blocked"
      min={0}
      formatValue={(value) => value.toFixed(0)}
      data={chart}
      series={[
        { key: 'running', label: 'Running' },
        { key: 'blocked', label: 'Blocked on I/O' },
      ]}
    />
  )
}

// Counts: whole numbers, with one decimal only for axis steps between them.
const integerValue = (value: number) =>
  Number.isInteger(value) ? value.toFixed(0) : value.toFixed(1)

// Kernel page size for the swap counters in /proc/vmstat.
const PAGE_BYTES = 4096

const vmEvent = (rows: Row[], key: string) =>
  num(
    rows.find((row) => row.key === key),
    'value',
  )

// Sum of the per-zone counters that share a prefix, such as pgscan_direct_.
const vmEvents = (rows: Row[], prefix: string) => {
  const matching = rows.filter((row) => String(row.key).startsWith(prefix))
  return matching.length ? matching.reduce((total, row) => total + (num(row, 'value') ?? 0), 0) : null
}

function SwapActivity() {
  const samples = useSamples()
  const { data, isPending } = useTable('linux_vm_event')
  const chart = rates(samples.data ?? [], data ?? [], (rows) => {
    const swapIn = vmEvent(rows, 'pswpin')
    const swapOut = vmEvent(rows, 'pswpout')
    return {
      in: swapIn == null ? null : (swapIn * PAGE_BYTES) / MIB,
      out: swapOut == null ? null : (swapOut * PAGE_BYTES) / MIB,
    }
  })
  return (
    <ChartPanel
      title="Swap activity"
      loading={isPending || samples.isPending}
      label="Swapped in and out (MiB/s)"
      min={0}
      formatValue={oneDecimal}
      data={chart}
      series={
        data?.length
          ? [
              { key: 'in', label: 'Swapped in' },
              { key: 'out', label: 'Swapped out' },
            ]
          : []
      }
    />
  )
}

function MajorFaults() {
  const samples = useSamples()
  const { data, isPending } = useTable('linux_vm_event')
  const chart = rates(samples.data ?? [], data ?? [], (rows) => ({
    major: vmEvent(rows, 'pgmajfault'),
  }))
  return (
    <ChartPanel
      title="Major page faults"
      loading={isPending || samples.isPending}
      label="Major page faults per second"
      min={0}
      formatValue={integerValue}
      data={chart}
      series={data?.length ? [{ key: 'major', label: 'Major faults' }] : []}
    />
  )
}

function PageReclaim() {
  const samples = useSamples()
  const { data, isPending } = useTable('linux_vm_event')
  const chart = rates(samples.data ?? [], data ?? [], (rows) => ({
    kswapd: vmEvents(rows, 'pgscan_kswapd'),
    // Kernel 4.8 and newer have one pgscan_direct counter; older kernels have
    // one per zone, such as pgscan_direct_normal. pgscan_direct_throttle
    // counts throttled processes, not pages.
    direct: vmEvents(
      rows.filter((row) => row.key !== 'pgscan_direct_throttle'),
      'pgscan_direct',
    ),
  }))
  return (
    <ChartPanel
      title="Page reclaim"
      loading={isPending || samples.isPending}
      label="Pages scanned for reclaim per second"
      min={0}
      formatValue={integerValue}
      data={chart}
      series={
        data?.length
          ? [
              { key: 'kswapd', label: 'kswapd, in the background' },
              { key: 'direct', label: 'Direct, by the allocating process' },
            ]
          : []
      }
    />
  )
}

function NetworkErrors() {
  const samples = useSamples()
  const { data, isPending } = useTable('linux_network_io')
  const total = (rows: Row[], column: string) =>
    rows.length ? rows.reduce((sum, row) => sum + (num(row, column) ?? 0), 0) : null
  const chart = rates(samples.data ?? [], data ?? [], (rows) => ({
    rx_dropped: total(rows, 'rx_dropped'),
    tx_dropped: total(rows, 'tx_dropped'),
    rx_errors: total(rows, 'rx_errors'),
    tx_errors: total(rows, 'tx_errors'),
  }))
  return (
    <ChartPanel
      title="Network drops and errors"
      loading={isPending || samples.isPending}
      label="Packets dropped and errors per second, all interfaces"
      min={0}
      formatValue={oneDecimal}
      data={chart}
      series={
        data?.length
          ? [
              { key: 'rx_dropped', label: 'Received, dropped' },
              { key: 'tx_dropped', label: 'Sent, dropped' },
              { key: 'rx_errors', label: 'Received, errors' },
              { key: 'tx_errors', label: 'Sent, errors' },
            ]
          : []
      }
    />
  )
}

// The five whole disks with the most completed I/O over the session.
function busiestDisks(rows: Row[], names: string[]) {
  const done = (name: string) => {
    const own = rows.filter((row) => row.kernel_name === name)
    const count = (row: Row | undefined) =>
      (num(row, 'reads_completed') ?? 0) + (num(row, 'writes_completed') ?? 0)
    return own.length < 2 ? 0 : count(own[own.length - 1]) - count(own[0])
  }
  return [...names]
    .map((name) => ({ name, done: done(name) }))
    .filter(({ done }) => done > 0)
    .sort((a, b) => b.done - a.done)
    .slice(0, 5)
    .map(({ name }) => name)
}

function DiskLatency({ direction }: { direction: 'read' | 'write' }) {
  const samples = useSamples()
  const disks = useDisks()
  const busiest = busiestDisks(disks.rows, disks.names)
  const time = `${direction}_milliseconds`
  const completed = `${direction}s_completed`
  const counters = (rows: Row[]): Values => {
    const values: Values = {}
    for (const name of busiest) {
      const row = rows.find((candidate) => candidate.kernel_name === name)
      values[`${name}:ms`] = num(row, time)
      values[`${name}:ios`] = num(row, completed)
    }
    return values
  }
  const chart = deltas(samples.data ?? [], disks.rows, counters, (delta) =>
    Object.fromEntries(
      busiest.map((name) => {
        const ms = delta[`${name}:ms`]
        const ios = delta[`${name}:ios`]
        return [name, ms == null || ios == null || ios === 0 ? null : ms / ios]
      }),
    ),
  )
  return (
    <ChartPanel
      title={direction === 'read' ? 'Disk read latency' : 'Disk write latency'}
      loading={disks.isPending || samples.isPending}
      label={
        direction === 'read'
          ? 'Average read latency, five busiest disks (ms)'
          : 'Average write latency, five busiest disks (ms)'
      }
      min={0}
      formatValue={oneDecimal}
      data={chart}
      series={busiest.map((name) => ({ key: name, label: name }))}
    />
  )
}

function DiskQueue() {
  const samples = useSamples()
  const disks = useDisks()
  const busiest = busiestDisks(disks.rows, disks.names)
  const chart = deltas(
    samples.data ?? [],
    disks.rows,
    (rows) =>
      Object.fromEntries(
        busiest.map((name) => [
          name,
          num(
            rows.find((row) => row.kernel_name === name),
            'weighted_io_milliseconds',
          ),
        ]),
      ),
    (delta, seconds) =>
      Object.fromEntries(
        Object.entries(delta).map(([name, ms]) => [name, ms == null ? null : ms / (seconds * 1000)]),
      ),
  )
  return (
    <ChartPanel
      title="Disk queue depth"
      loading={disks.isPending || samples.isPending}
      label="Average requests in flight, five busiest disks"
      min={0}
      formatValue={oneDecimal}
      data={chart}
      series={busiest.map((name) => ({ key: name, label: name }))}
    />
  )
}

function CgroupMemory() {
  const samples = useSamples()
  const { data, isPending } = useTable('linux_cgroup_usage')
  const recorded = (data ?? []).some(
    (row) => num(row, 'memory_current_bytes') != null || num(row, 'swap_current_bytes') != null,
  )
  const chart = points(samples.data ?? [], data ?? [], (rows) => {
    const memory = num(rows[0], 'memory_current_bytes')
    const swap = num(rows[0], 'swap_current_bytes')
    return {
      memory: memory == null ? null : memory / GIB,
      swap: swap == null ? null : swap / GIB,
    }
  })
  return (
    <ChartPanel
      title="Cgroup memory"
      loading={isPending || samples.isPending}
      label="Memory and swap charged to the cgroup (GiB)"
      min={0}
      formatValue={oneDecimal}
      data={chart}
      series={
        recorded
          ? [
              { key: 'memory', label: 'Memory' },
              { key: 'swap', label: 'Swap' },
            ]
          : []
      }
    />
  )
}

function CgroupIo() {
  const samples = useSamples()
  const { data, isPending } = useTable('linux_cgroup_io')
  const outcomes = useOutcomes()
  const label = 'Cgroup I/O, all devices (MiB/s)'
  if (unsupportedCode(outcomes, 'proc.cgroup.io')) {
    return (
      <NotSupportedPanel title="Cgroup I/O" label={label} on="this host">
        The probe's cgroup has no io.stat file (cgroup v2) or blkio.io_service_bytes_recursive file (cgroup v1).
      </NotSupportedPanel>
    )
  }
  const total = (rows: Row[], column: string) =>
    rows.length ? rows.reduce((sum, row) => sum + (num(row, column) ?? 0), 0) / MIB : null
  const chart = rates(samples.data ?? [], data ?? [], (rows) => ({
    read: total(rows, 'read_bytes'),
    write: total(rows, 'write_bytes'),
  }))
  return (
    <ChartPanel
      title="Cgroup I/O"
      loading={isPending || samples.isPending}
      label={label}
      min={0}
      formatValue={oneDecimal}
      data={chart}
      series={
        data?.length
          ? [
              { key: 'read', label: 'Read' },
              { key: 'write', label: 'Write' },
            ]
          : []
      }
    />
  )
}

const LIMIT_EVENTS = ['cpu.nr_throttled', 'memory.high', 'memory.max', 'memory.oom_kill', 'pids.max']

function CgroupEvents() {
  const samples = useSamples()
  const { data, isPending } = useTable('linux_cgroup_event')
  const outcomes = useOutcomes()
  const label = 'Cgroup limit events per interval'
  if (unsupportedCode(outcomes, 'proc.cgroup.events')) {
    return (
      <NotSupportedPanel title="Cgroup events" label={label} on="this host">
        The probe's cgroup has no cpu.stat or memory.events counters.
      </NotSupportedPanel>
    )
  }
  const keyOf = (row: Row) => `${row.controller}.${row.event_name}`
  // The counters the report explains, when the cgroup has them. Cgroup v1
  // records only CPU usage, so it shows that.
  const present = [...new Set((data ?? []).map(keyOf))]
  const explained = LIMIT_EVENTS.filter((key) => present.includes(key))
  const keys = (explained.length ? explained : present).slice(0, 5)
  const chart = deltas(
    samples.data ?? [],
    data ?? [],
    (rows) =>
      Object.fromEntries(
        keys.map((key) => [
          key,
          num(
            rows.find((row) => keyOf(row) === key),
            'cumulative_value',
          ),
        ]),
      ),
    (delta) => delta,
  )
  return (
    <ChartPanel
      title="Cgroup events"
      loading={isPending || samples.isPending}
      label={label}
      min={0}
      formatValue={integerValue}
      data={chart}
      series={keys.map((key) => ({ key, label: key }))}
    />
  )
}

export function System() {
  return (
    <Section title="System">
      <PanelGrid id="system" label="System panels">
        <Slot id="cpu-utilization" size={8}>
          <CpuUtilization />
        </Slot>
        <Slot id="load-average" size={4}>
          <LoadAverage />
        </Slot>
        <Slot id="memory" size={8}>
          <Memory />
        </Slot>
        <Slot id="swap-in-use" size={4}>
          <Swap />
        </Slot>
        <Slot id="disk-throughput" size={6}>
          <DiskThroughput />
        </Slot>
        <Slot id="disk-utilization" size={6}>
          <DiskUtilization />
        </Slot>
        <Slot id="network-throughput" size={6}>
          <NetworkThroughput />
        </Slot>
        <Slot id="pressure-stall" size={6}>
          <PressureStall />
        </Slot>
        <Slot id="filesystem-usage" size={6}>
          <FilesystemUsage />
        </Slot>
        <Slot id="processes" size={6}>
          <Processes />
        </Slot>
        <Slot id="swap-activity" size={6}>
          <SwapActivity />
        </Slot>
        <Slot id="major-page-faults" size={6}>
          <MajorFaults />
        </Slot>
        <Slot id="page-reclaim" size={6}>
          <PageReclaim />
        </Slot>
        <Slot id="network-drops-and-errors" size={6}>
          <NetworkErrors />
        </Slot>
        <Slot id="disk-read-latency" size={6}>
          <DiskLatency direction="read" />
        </Slot>
        <Slot id="disk-write-latency" size={6}>
          <DiskLatency direction="write" />
        </Slot>
        <Slot id="disk-queue-depth" size={6}>
          <DiskQueue />
        </Slot>
        <Slot id="cgroup-memory" size={6}>
          <CgroupMemory />
        </Slot>
        <Slot id="cgroup-i-o" size={6}>
          <CgroupIo />
        </Slot>
        <Slot id="cgroup-events" size={6}>
          <CgroupEvents />
        </Slot>
      </PanelGrid>
    </Section>
  )
}
