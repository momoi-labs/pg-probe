import { missingReason, sum, useOutcomes, useTable } from '../artifact.ts'
import { useT } from '../i18n.ts'
import { rates, useSamples } from '../series.ts'
import { ChartPanel, GapPanel, Note, Panel, PanelGrid, Slot } from './panels.tsx'

function RowsPanel() {
  const samples = useSamples()
  const { data, isPending } = useTable('postgres_database_stats')
  const chart = rates(samples.data ?? [], data ?? [], (rows) =>
    rows.length
      ? {
          read: sum(rows, 'tup_returned'),
          written: sum(rows, 'tup_inserted') + sum(rows, 'tup_updated') + sum(rows, 'tup_deleted'),
        }
      : { read: null, written: null },
  )
  return (
    <ChartPanel
      title="Rows"
      loading={isPending || samples.isPending}
      label="Rows per second"
      min={0}
      formatValue={(value) => value.toFixed(0)}
      data={chart}
      series={[
        { key: 'read', label: 'Read' },
        { key: 'written', label: 'Written' },
      ]}
    />
  )
}

const STATEMENTS_GAP =
  'Needs a new postgres.stats.statements collector, after the decision on whether query text may enter the artifact.'

function QuerySource() {
  const t = useT()
  const { data, isPending } = useTable('postgres_extension')
  const outcomes = useOutcomes()
  const databases = (data ?? [])
    .filter((row) => row.extension_name === 'pg_stat_statements')
    .map((row) => String(row.database_name))
  const reason = missingReason(outcomes, 'postgres.extensions')
  return (
    <Panel title="Query source" sub="stated, never guessed" loading={isPending}>
      <Note>
        {databases.length > 0
          ? t('pg_stat_statements is installed in {databases}. The artifact does not read it yet.', {
              databases: databases.join(', '),
            })
          : t('pg_stat_statements is not installed in any database the probe visited.')}
        {reason && ` ${t('Some databases were not visited: {reason}.', { reason })}`}
      </Note>
    </Panel>
  )
}

export function Queries() {
  const t = useT()
  return (
    <PanelGrid id="queries" label="Query panels">
      <Slot id="query-source" size={12}>
        <QuerySource />
      </Slot>
      <Slot id="query-latency" size={6}>
        <GapPanel
          title="Query latency"
          sub="percentiles across the window"
          gap="blocked"
        >
          {t(
            'pg_stat_statements stores totals and means, never a distribution. Percentiles need the server log, which the probe does not read.',
          )}
        </GapPanel>
      </Slot>
      <Slot id="query-volume" size={6}>
        <GapPanel title="Query volume" sub="calls per second" gap="no collector">
          {t(STATEMENTS_GAP)}
        </GapPanel>
      </Slot>
      <Slot id="rows" size={6}>
        <RowsPanel />
      </Slot>
      <Slot id="time-split" size={6}>
        <GapPanel
          title="Time split"
          sub="non-I/O against block I/O"
          gap="no collector"
        >
          {`${t(STATEMENTS_GAP)} ${t('It also needs track_io_timing on the server.')}`}
        </GapPanel>
      </Slot>
      <Slot id="most-expensive-statements" size={12}>
        <GapPanel
          title="Most expensive statements"
          sub="by total time in the window"
          gap="no collector"
        >
          {t(STATEMENTS_GAP)}
        </GapPanel>
      </Slot>
    </PanelGrid>
  )
}
