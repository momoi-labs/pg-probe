import { DashboardGrid } from '@momoi-labs/kiso-react'
import { missingReason, useOutcomes, useTable } from '../artifact.ts'
import { GapPanel, Note, Panel } from './panels.tsx'

const STATEMENTS_GAP =
  'Needs a new postgres.stats.statements collector, after the decision on whether query text may enter the artifact.'

function QuerySource() {
  const { data, isPending } = useTable('postgres_extension')
  const outcomes = useOutcomes()
  const databases = (data ?? [])
    .filter((row) => row.extension_name === 'pg_stat_statements')
    .map((row) => String(row.database_name))
  const reason = missingReason(outcomes, 'postgres.extensions')
  return (
    <Panel title="Query source" sub="stated, never guessed" span={12} loading={isPending}>
      <Note>
        {databases.length > 0
          ? `pg_stat_statements is installed in ${databases.join(', ')}. The artifact does not read it yet.`
          : 'pg_stat_statements is not installed in any database the probe visited.'}
        {reason && ` Some databases were not visited: ${reason}.`}
      </Note>
    </Panel>
  )
}

export function Queries() {
  return (
    <DashboardGrid>
      <QuerySource />
      <GapPanel
        title="Query latency"
        sub="percentiles across the window"
        span={6}
        gap="blocked"
      >
        pg_stat_statements stores totals and means, never a distribution. Percentiles need the
        server log, which the probe does not read.
      </GapPanel>
      <GapPanel title="Query volume" sub="calls per second" span={6} gap="no collector">
        {STATEMENTS_GAP}
      </GapPanel>
      <GapPanel title="Rows" sub="read and written per second" span={6} gap="needs sampling">
        postgres_database_stats has the tup_* counters. A rate needs two samples.
      </GapPanel>
      <GapPanel
        title="Time split"
        sub="non-I/O against block I/O"
        span={6}
        gap="no collector"
      >
        {STATEMENTS_GAP} It also needs track_io_timing on the server.
      </GapPanel>
      <GapPanel
        title="Most expensive statements"
        sub="by total time in the window"
        span={12}
        gap="no collector"
      >
        {STATEMENTS_GAP}
      </GapPanel>
    </DashboardGrid>
  )
}
