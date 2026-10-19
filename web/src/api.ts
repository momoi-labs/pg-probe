import { queryOptions } from '@tanstack/react-query'

// Response shapes of src/review/mod.rs. Written by hand until the project
// chooses how to generate them from the Rust types.

export type SnapshotListing = {
  id: string
  file_name: string
  size_bytes: number
  snapshot_id: string
  started_at_utc: string
  finished_at_utc: string
  status: string
  hostname: string | null
}

export type Row = Record<string, string | number | null>

export type TableRows = {
  table: string
  columns: { name: string; type: string; nullable: boolean; primary_key: boolean }[]
  rows: Row[]
  truncated: boolean
}

export type SnapshotDetail = {
  snapshot: SnapshotListing
  collector_outcomes: Row[]
}

async function getJson<T>(path: string): Promise<T> {
  const response = await fetch(path)
  if (!response.ok) {
    const body = await response.json().catch(() => null)
    throw new Error(body?.error ?? `${response.status} ${response.statusText}`)
  }
  return response.json()
}

// The API accepts the file name with or without the .sqlite.gz suffix. Routes
// use the short form.
export const routeId = (listing: SnapshotListing) =>
  listing.id.replace(/\.sqlite\.gz$/, '')

// Newest capture first. The API lists files in name order.
const newestFirst = (snapshots: SnapshotListing[]) =>
  [...snapshots].sort((a, b) => b.started_at_utc.localeCompare(a.started_at_utc))

export const snapshotsQuery = queryOptions({
  queryKey: ['snapshots'],
  queryFn: async () =>
    newestFirst((await getJson<{ snapshots: SnapshotListing[] }>('/api/snapshots')).snapshots),
})

export const snapshotQuery = (id: string) =>
  queryOptions({
    queryKey: ['snapshot', id],
    queryFn: () => getJson<SnapshotDetail>(`/api/snapshots/${encodeURIComponent(id)}`),
  })

export type Sample = { sample_id: number; taken_at_utc: string }

export const samplesQuery = (id: string) =>
  queryOptions({
    queryKey: ['samples', id],
    queryFn: () =>
      getJson<{ samples: Sample[] }>(`/api/snapshots/${encodeURIComponent(id)}/samples`),
    select: (data) => data.samples,
  })

// Filters keep equality on a column, for example { cpu_id: '-1' }.
export const tableQuery = (id: string, table: string, filters: Record<string, string> = {}) =>
  queryOptions({
    queryKey: ['table', id, table, filters],
    queryFn: async () => {
      const search = new URLSearchParams(filters).toString()
      try {
        return await getJson<TableRows>(
          `/api/snapshots/${encodeURIComponent(id)}/tables/${encodeURIComponent(table)}${search ? `?${search}` : ''}`,
        )
      } catch (error) {
        // Artifacts from an older schema lack newer tables. Their panels show
        // the value as not recorded, as for an empty table.
        if (error instanceof Error && error.message === 'table not found') {
          return { table, columns: [], rows: [], truncated: false }
        }
        throw error
      }
    },
    select: (data) => data.rows,
  })
