import { useQuery } from '@tanstack/react-query'
import { useParams } from '@tanstack/react-router'
import { snapshotQuery, tableQuery, type Row } from './api.ts'

export function useTable(table: string, filters?: Record<string, string>) {
  const { snapshotId } = useParams({ from: '/$snapshotId' })
  return useQuery(tableQuery(snapshotId, table, filters))
}

// Rows written before the sample dimension carry no sample_id. They belong
// to the single sample such an artifact holds.
export const sampleOf = (row: Row) => (typeof row.sample_id === 'number' ? row.sample_id : 1)

// The rows of the last sample. Invariant tables have no sample_id, so all of
// their rows come back.
export function latest(rows: Row[] | undefined): Row[] {
  if (!rows?.length) return []
  const last = Math.max(...rows.map(sampleOf))
  return rows.filter((row) => sampleOf(row) === last)
}

export function useOutcomes() {
  const { snapshotId } = useParams({ from: '/$snapshotId' })
  return useQuery(snapshotQuery(snapshotId)).data?.collector_outcomes ?? []
}

export const num = (row: Row | undefined, column: string) => {
  const value = row?.[column]
  return typeof value === 'number' ? value : null
}

export const sum = (rows: Row[], column: string) =>
  rows.reduce((total, row) => total + (num(row, column) ?? 0), 0)

export const integer = (value: number) => value.toLocaleString('en-US')

// Why a source has no rows: the first outcome for it that did not collect,
// for example "unsupported (psi_read_failed)". Null when it collected or has
// no outcome at all.
export function missingReason(outcomes: Row[], source: string) {
  const failed = outcomes.find(
    (row) => String(row.source).startsWith(source) && row.status !== 'collected',
  )
  if (!failed) return null
  return failed.error_code == null
    ? String(failed.status)
    : `${failed.status} (${failed.error_code})`
}

export const collected = (outcomes: Row[], source: string) =>
  outcomes.some((row) => row.source === source && row.status === 'collected')

// The error code of a source the collector marked unsupported on the observed
// server or host, for example "not_in_server_version". Null otherwise.
export function unsupportedCode(outcomes: Row[], source: string) {
  const row = outcomes.find((outcome) => outcome.source === source && outcome.status === 'unsupported')
  return row ? String(row.error_code ?? 'unsupported') : null
}

// server_version_num and the major version for copy, such as "9.3" or "16".
// Null without a PostgreSQL target.
export function useServerVersion() {
  const { data, isPending } = useTable('postgres_target')
  const versionNum = num(data?.[0], 'server_version_num')
  const major =
    versionNum == null
      ? null
      : versionNum >= 100000
        ? String(Math.floor(versionNum / 10000))
        : `${Math.floor(versionNum / 10000)}.${Math.floor(versionNum / 100) % 100}`
  return { versionNum, major, isPending }
}
