import { BarGauge, DashboardGrid, Disclosure, KV, KVKey, KVValue, Meter } from '@momoi-labs/kiso-react'
import { integer, missingReason, num, useOutcomes, useTable } from '../artifact.ts'
import { bytes } from '../format.tsx'
import { GapPanel, Note, Panel } from './panels.tsx'

const gib = (value: number) => `${(value / 1024 ** 3).toFixed(1)} GiB`

function LoadAverage() {
  const { data, isPending } = useTable('linux_scheduler')
  const row = data?.[0]
  return (
    <Panel title="Load average" sub="1m / 5m / 15m" span={4} loading={isPending}>
      {row ? (
        <KV>
          {(['1m', '5m', '15m'] as const).map((window) => (
            <div key={window}>
              <KVKey>{window}</KVKey>
              <KVValue>{num(row, `load_avg_${window}`)?.toFixed(2) ?? 'NULL'}</KVValue>
            </div>
          ))}
        </KV>
      ) : (
        <Note>Not recorded in this artifact.</Note>
      )}
    </Panel>
  )
}

function Memory() {
  const { data, isPending } = useTable('linux_memory')
  const value = (key: string) => num(data?.find((row) => row.key === key), 'value_bytes')
  const total = value('MemTotal')
  const free = value('MemFree')
  const buffers = value('Buffers')
  const cached = value('Cached')
  const shmem = value('Shmem')
  const reclaimable = value('SReclaimable')
  const known =
    total != null &&
    free != null &&
    buffers != null &&
    cached != null &&
    shmem != null &&
    reclaimable != null
  return (
    <Panel title="Memory" sub="split of MemTotal" span={8} loading={isPending}>
      {known ? (
        <>
          <BarGauge
            label={`Memory by use, of ${gib(total)} MemTotal`}
            max={total}
            formatValue={gib}
            rows={[
              {
                key: 'used',
                label: 'Used',
                value: total - free - buffers - cached - reclaimable,
              },
              { key: 'shmem', label: 'Shared memory', value: shmem },
              { key: 'cache', label: 'Page cache', value: cached - shmem + reclaimable },
              { key: 'buffers', label: 'Buffers', value: buffers },
              { key: 'free', label: 'Free', value: free },
            ]}
          />
          <Note>
            From /proc/meminfo. Page cache is Cached − Shmem + SReclaimable, because Cached
            already includes Shmem. Used is MemTotal − MemFree − Buffers − Cached −
            SReclaimable. Shared memory includes shared_buffers.
          </Note>
        </>
      ) : (
        <Note>Not recorded in this artifact.</Note>
      )}
    </Panel>
  )
}

function Swap() {
  const { data, isPending } = useTable('linux_memory')
  const value = (key: string) => num(data?.find((row) => row.key === key), 'value_bytes')
  const total = value('SwapTotal')
  const free = value('SwapFree')
  let content
  if (total == null || free == null) content = <Note>Not recorded in this artifact.</Note>
  else if (total === 0) content = <Note>No swap is configured.</Note>
  else {
    const used = bytes(total - free)
    const size = bytes(total)
    content = (
      <>
        <Meter
          label="Swap used"
          value={total - free}
          max={total}
          valueText={`${used.number} ${used.unit} of ${size.number} ${size.unit}`}
        />
        <Note>SwapTotal − SwapFree, from /proc/meminfo</Note>
      </>
    )
  }
  return (
    <Panel title="Swap in use" span={4} loading={isPending}>
      {content}
    </Panel>
  )
}

function PressureStall() {
  const { data, isPending } = useTable('linux_psi')
  const outcomes = useOutcomes()
  const some = data?.filter((row) => row.scope === 'some') ?? []
  const reason = missingReason(outcomes, 'proc.pressure')
  return (
    <Panel title="Pressure stall" sub="some, 10s window" span={6} loading={isPending}>
      {some.length > 0 ? (
        <BarGauge
          label="Share of time some tasks stalled, avg10"
          max={100}
          formatValue={(value) => `${value.toFixed(1)}%`}
          rows={some.map((row) => ({
            key: String(row.resource),
            label: String(row.resource),
            value: num(row, 'avg10'),
          }))}
        />
      ) : (
        <Note>
          Not recorded in this artifact
          {reason ? `: proc.pressure is ${reason}. Older kernels and some containers have no PSI.` : '.'}
        </Note>
      )}
    </Panel>
  )
}

function FilesystemUsage() {
  const { data, isPending } = useTable('linux_mount')
  const sized = data?.filter((row) => num(row, 'total_bytes') != null).length ?? 0
  return (
    <GapPanel title="Filesystem usage" sub="point in time" span={6} gap="needs schema change">
      {isPending
        ? 'Loading.'
        : `linux_mount has no mount path, so the console cannot name the data or WAL volume. ${sized} of ${data?.length ?? 0} mounts in this artifact have a size.`}
    </GapPanel>
  )
}

function Processes() {
  const { data, isPending } = useTable('linux_scheduler')
  const row = data?.[0]
  const fields = [
    ['Running', 'procs_running'],
    ['Blocked on I/O', 'procs_blocked'],
    ['Total tasks', 'total_tasks'],
  ] as const
  return (
    <Panel title="Processes" sub="running and blocked" span={6} loading={isPending}>
      {row ? (
        <>
          <KV>
            {fields.map(([label, column]) => {
              const value = num(row, column)
              return (
                <div key={column}>
                  <KVKey>{label}</KVKey>
                  <KVValue>{value == null ? 'NULL' : integer(value)}</KVValue>
                </div>
              )
            })}
          </KV>
          <Note>procs_running and procs_blocked from /proc/stat, tasks from /proc/loadavg</Note>
        </>
      ) : (
        <Note>Not recorded in this artifact.</Note>
      )}
    </Panel>
  )
}

const NEEDS_SAMPLING = 'needs sampling'

export function System() {
  return (
    <Disclosure summary="System" open>
      <DashboardGrid>
        <GapPanel title="CPU utilisation" sub="by state" span={8} gap={NEEDS_SAMPLING}>
          linux_cpu_time has the per-CPU tick counters. A share of time needs the change between
          two samples, and the artifact holds one.
        </GapPanel>
        <LoadAverage />
        <Memory />
        <Swap />
        <GapPanel title="Disk throughput" sub="per volume" span={6} gap={NEEDS_SAMPLING}>
          linux_block_io has the sector counters. Bytes per second needs two samples.
        </GapPanel>
        <GapPanel
          title="Disk utilisation"
          sub="share of time the device was busy"
          span={6}
          gap={NEEDS_SAMPLING}
        >
          linux_block_io has io_milliseconds. Busy time needs two samples.
        </GapPanel>
        <GapPanel title="Network throughput" sub="primary interface" span={6} gap="no collector">
          No collector reads /proc/net/dev yet. The plan is a new linux.network collector.
        </GapPanel>
        <PressureStall />
        <FilesystemUsage />
        <Processes />
      </DashboardGrid>
    </Disclosure>
  )
}
