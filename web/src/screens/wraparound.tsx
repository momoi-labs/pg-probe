import {
  Meter,
  Sparkline,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@momoi-labs/kiso-react'
import type { Row } from '../api.ts'
import { integer, latest, num, sampleOf, unsupportedCode, useOutcomes, useTable } from '../artifact.ts'
import { Unknown } from '../format.tsx'
import { useT } from '../i18n.ts'
import { useSamples } from '../series.ts'
import { Note, NotSupportedPanel, Panel, PanelGrid, Section, Slot } from './panels.tsx'

// Segment files in pg_multixact/members when the member space is full: 2^32
// members at 52,352 members per file.
const MEMBER_SEGMENTS = 82040
const TOP = 10
const notRecorded =
  'Not recorded in this artifact. Captures before schema version 4 have no wraparound state.'

type Tries = {
  database: string
  relation: string
  toastOf: string | null
  // One try is one vacuum, known by the time it started.
  tries: Set<string>
  wraparound: Set<string>
  samples: Set<number>
  first?: Row
  last?: Row
}

// Vacuums that finished on the table, both kinds. Null when neither counter
// is known.
const finishedCount = (row: Row | undefined) => {
  const auto = num(row, 'autovacuum_count')
  const manual = num(row, 'vacuum_count')
  return auto == null && manual == null ? null : (auto ?? 0) + (manual ?? 0)
}

// Time from the last autovacuum to the read of the table, in the largest
// unit that keeps two digits or more.
function since(row: Row | undefined) {
  if (!row) return '—'
  if (row.last_autovacuum == null) return 'NULL'
  const minutes = (Date.parse(String(row.collected_at)) - Date.parse(String(row.last_autovacuum))) / 60000
  if (Number.isNaN(minutes)) return '—'
  if (minutes >= 2880) return `${Math.floor(minutes / 1440)} d`
  if (minutes >= 120) return `${Math.floor(minutes / 60)} h`
  return `${Math.max(0, Math.floor(minutes))} min`
}

function VacuumTries() {
  const t = useT()
  const samples = useSamples()
  const tables = useTable('postgres_table_freeze')
  const vacuums = useTable('postgres_vacuum_running')
  const hidden = unsupportedCode(useOutcomes(), 'postgres.freeze.vacuums') === 'activity_not_visible'
  const isPending = samples.isPending || tables.isPending || vacuums.isPending

  const byTable = new Map<string, Tries>()
  const entry = (database: string, relation: string) => {
    const key = `${database}.${relation}`
    let found = byTable.get(key)
    if (!found) {
      found = { database, relation, toastOf: null, tries: new Set(), wraparound: new Set(), samples: new Set() }
      byTable.set(key, found)
    }
    return found
  }
  for (const row of tables.data ?? []) {
    const found = entry(String(row.database_name), `${row.schema_name}.${row.relation_name}`)
    if (row.toast_of != null) found.toastOf = String(row.toast_of)
    if (!found.first || sampleOf(row) < sampleOf(found.first)) found.first = row
    if (!found.last || sampleOf(row) > sampleOf(found.last)) found.last = row
  }
  for (const row of vacuums.data ?? []) {
    if (row.relation == null) continue
    // A manual vacuum may name its table without the schema. The capture
    // does not know the search_path of that session, so the name stays a row
    // of its own instead of joining a table that may be another one.
    const found = entry(String(row.database_name), String(row.relation))
    found.tries.add(String(row.started_at))
    if (row.wraparound === 1) found.wraparound.add(String(row.started_at))
    found.samples.add(sampleOf(row))
  }
  const rows = [...byTable.values()]
    .sort(
      (a, b) =>
        b.tries.size - a.tries.size ||
        (num(b.last, 'frozenxid_age') ?? 0) - (num(a.last, 'frozenxid_age') ?? 0),
    )
    .slice(0, TOP)
  const readOnce = rows.some((row) => row.first === row.last)
  // With one database in view, its name goes in the badge and leaves the rows.
  const databases = new Set(rows.map((row) => row.database))
  const only = databases.size === 1 ? rows[0].database : undefined
  const nameOf = (row: Tries) => (only ? row.relation : `${row.database}.${row.relation}`)

  let content
  if (isPending) {
    content = null
  } else if (rows.length === 0) {
    content = <Note>{t(notRecorded)}</Note>
  } else {
    content = (
      <>
        <Table density="compact">
          <TableHeader>
            <TableRow>
              <TableHead>{t('Table')}</TableHead>
              <TableHead className="num">{t('Tries seen')}</TableHead>
              <TableHead className="num">{t('For wraparound')}</TableHead>
              <TableHead className="num">{t('Finished')}</TableHead>
              <TableHead>{t('Vacuum running, per sample')}</TableHead>
              <TableHead className="num col-tight">{t('xid age')}</TableHead>
              <TableHead className="num">{t('Since last autovacuum')}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row) => {
              const before = row.first === row.last ? null : finishedCount(row.first)
              const after = finishedCount(row.last)
              // A counter that went down was reset, so the change is unknown.
              const finished = before == null || after == null || after < before ? null : after - before
              const age = num(row.last, 'frozenxid_age')
              return (
                <TableRow key={`${row.database}.${row.relation}`}>
                  <TableCell>
                    <code>{nameOf(row)}</code>
                    {row.toastOf && (
                      <>
                        {' '}
                        <span className="muted">{t('TOAST of {table}', { table: row.toastOf })}</span>
                      </>
                    )}
                  </TableCell>
                  <TableCell className="num">{row.tries.size}</TableCell>
                  <TableCell className="num">{row.wraparound.size}</TableCell>
                  <TableCell className="num">
                    {finished == null ? <Unknown reason={t('one read of the table')} /> : finished}
                  </TableCell>
                  <TableCell>
                    <div className="trend">
                      <Sparkline
                        values={(samples.data ?? []).map((sample) =>
                          row.samples.has(sample.sample_id) ? 1 : 0,
                        )}
                        min={0}
                        max={1}
                        height={18}
                        chartLine="stepped"
                        fill
                      />
                    </div>
                  </TableCell>
                  <TableCell className="num col-tight">
                    {age == null ? <Unknown reason={t('table not read')} /> : integer(age)}
                  </TableCell>
                  <TableCell className="num">{since(row.last)}</TableCell>
                </TableRow>
              )
            })}
          </TableBody>
        </Table>
        {readOnce && (
          <Note>
            {t('Finished is unknown for a table the capture read once. The capture reads tables once per minute.')}
          </Note>
        )}
        {hidden && (
          <Note>
            {t('The capture role cannot see the sessions of other roles, so their vacuums are missing here.')}
          </Note>
        )}
      </>
    )
  }
  return (
    <Panel
      title="Vacuum tries by table"
      sub="whole capture, top 10 by tries seen, then by xid age"
      badge={only}
      loading={isPending}
    >
      {content}
    </Panel>
  )
}

function MemberSpace() {
  const t = useT()
  const members = useTable('postgres_multixact_members')
  const outcomes = useOutcomes()
  const code = unsupportedCode(outcomes, 'postgres.freeze.members')
  const title = 'Multixact member space'
  if (code === 'not_in_server_version') {
    return (
      <NotSupportedPanel title={title} label={title} on="this PostgreSQL version">
        From PostgreSQL 19 the multixact member space has no limit to reach.
      </NotSupportedPanel>
    )
  }
  const rows = [...(members.data ?? [])].sort((a, b) => sampleOf(a) - sampleOf(b))
  const first = rows[0]
  const last = rows.at(-1)
  const files = num(last, 'segment_files')
  const minutes =
    first && last ? (Date.parse(String(last.collected_at)) - Date.parse(String(first.collected_at))) / 60000 : 0
  const growth = minutes > 0 && files != null ? (files - (num(first, 'segment_files') ?? files)) / minutes : null
  let content
  if (members.isPending) {
    content = null
  } else if (files == null) {
    content = (
      <Note>
        {code === 'members_not_permitted'
          ? t(
              'Not recorded: the capture role may not list pg_multixact/members. It needs a superuser, or from PostgreSQL 11 EXECUTE on pg_ls_dir.',
            )
          : t(notRecorded)}
      </Note>
    )
  } else {
    content = (
      <>
        <Meter
          label={t('Segment files in pg_multixact/members')}
          value={files}
          max={MEMBER_SEGMENTS}
          valueText={t('{files} of {limit} ({percent}%)', {
            files: integer(files),
            limit: integer(MEMBER_SEGMENTS),
            percent: ((files / MEMBER_SEGMENTS) * 100).toFixed(1),
          })}
          labels={{ notCollected: t('Not collected') }}
        />
        <Note>
          {growth == null
            ? t('The growth needs more than one sample.')
            : t('{sign}{rate} files per minute over the capture', {
                sign: growth > 0 ? '+' : '',
                rate: growth.toFixed(1),
              })}
        </Note>
      </>
    )
  }
  return (
    <Panel title={title} sub="last sample" loading={members.isPending}>
      {content}
    </Panel>
  )
}

function OldestDatabases() {
  const t = useT()
  const { data, isPending } = useTable('postgres_database_freeze')
  const rows = latest(data).sort(
    (a, b) => (num(b, 'frozenxid_age') ?? 0) - (num(a, 'frozenxid_age') ?? 0),
  )
  const noAge = rows.length > 0 && rows.every((row) => row.minmxid_age == null)
  let content
  if (isPending) {
    content = null
  } else if (rows.length === 0) {
    content = <Note>{t(notRecorded)}</Note>
  } else {
    content = (
      <>
        <Table density="compact">
          <TableHeader>
            <TableRow>
              <TableHead>{t('Database')}</TableHead>
              <TableHead className="num">{t('xid age')}</TableHead>
              <TableHead className="num">{t('Multixact age')}</TableHead>
              <TableHead className="num">{t('Multixact rank')}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row) => {
              const age = num(row, 'minmxid_age')
              const rank = num(row, 'minmxid_rank')
              return (
                <TableRow key={String(row.database_name)}>
                  <TableCell>
                    <code>{String(row.database_name)}</code>
                    {row.allows_connections === 0 && (
                      <>
                        {' '}
                        <span className="muted">{t('does not accept connections')}</span>
                      </>
                    )}
                  </TableCell>
                  <TableCell className="num">{integer(num(row, 'frozenxid_age') ?? 0)}</TableCell>
                  <TableCell className="num">
                    {age == null ? <Unknown reason={t('no mxid_age before PostgreSQL 9.5')} /> : integer(age)}
                  </TableCell>
                  <TableCell className="num">{rank ?? '—'}</TableCell>
                </TableRow>
              )
            })}
          </TableBody>
        </Table>
        {noAge && (
          <Note>
            {t('PostgreSQL before 9.5 has no multixact age. The rank gives the order: 1 is the oldest.')}
          </Note>
        )}
      </>
    )
  }
  return (
    <Panel title="Oldest databases" sub="last sample, by xid age" loading={isPending}>
      {content}
    </Panel>
  )
}

export function Wraparound() {
  return (
    <Section title="Wraparound">
      <PanelGrid id="wraparound" label="Wraparound panels">
        <Slot id="vacuum-tries-by-table" size={12}>
          <VacuumTries />
        </Slot>
        <Slot id="multixact-member-space" size={6}>
          <MemberSpace />
        </Slot>
        <Slot id="oldest-databases" size={6}>
          <OldestDatabases />
        </Slot>
      </PanelGrid>
    </Section>
  )
}
