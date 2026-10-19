import { useQuery } from '@tanstack/react-query'
import { useParams } from '@tanstack/react-router'
import { snapshotQuery, tableQuery, type Row } from './api.ts'

export function useTable(table: string) {
  const { snapshotId } = useParams({ from: '/$snapshotId' })
  return useQuery(tableQuery(snapshotId, table))
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
