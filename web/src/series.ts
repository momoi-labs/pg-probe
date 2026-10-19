import type { ChartSample } from '@momoi-labs/kiso-react'
import { useQuery } from '@tanstack/react-query'
import { useParams } from '@tanstack/react-router'
import { samplesQuery, type Row, type Sample } from './api.ts'
import { sampleOf } from './artifact.ts'

export type Values = Record<string, number | null>

export function useSamples() {
  const { snapshotId } = useParams({ from: '/$snapshotId' })
  return useQuery(samplesQuery(snapshotId))
}

function bySample(rows: Row[]): Map<number, Row[]> {
  const groups = new Map<number, Row[]>()
  for (const row of rows) {
    const id = sampleOf(row)
    groups.set(id, [...(groups.get(id) ?? []), row])
  }
  return groups
}

const timestamp = (sample: Sample) => Date.parse(sample.taken_at_utc)

// One chart point per sample, from the rows of that sample. Used for gauges
// such as load or memory. A sample without rows gets an empty list, so the
// reader returns nulls and the chart draws a gap.
export function points(
  samples: Sample[],
  rows: Row[],
  read: (rows: Row[]) => Values,
): ChartSample[] {
  const groups = bySample(rows)
  return samples.map((sample) => ({
    timestamp: timestamp(sample),
    values: read(groups.get(sample.sample_id) ?? []),
  }))
}

// One chart point per sample from the change since the previous sample.
// `combine` gets each counter's delta and the elapsed seconds. The first
// sample has no previous one, and a negative delta means a counter reset
// (a restart or a statistics reset), so both are gaps, never a value.
export function deltas(
  samples: Sample[],
  rows: Row[],
  read: (rows: Row[]) => Values,
  combine: (delta: Values, seconds: number) => Values,
): ChartSample[] {
  const groups = bySample(rows)
  const counters = samples.map((sample) => read(groups.get(sample.sample_id) ?? []))
  return samples.map((sample, index) => {
    const keys = Object.keys(counters[index])
    if (index === 0) {
      return { timestamp: timestamp(sample), values: Object.fromEntries(keys.map((k) => [k, null])) }
    }
    const seconds = (timestamp(sample) - timestamp(samples[index - 1])) / 1000
    const delta: Values = {}
    for (const key of keys) {
      const now = counters[index][key]
      const before = counters[index - 1][key]
      delta[key] = now == null || before == null || now < before ? null : now - before
    }
    return { timestamp: timestamp(sample), values: combine(delta, seconds) }
  })
}

// Per-second rate of each counter between samples.
export function rates(samples: Sample[], rows: Row[], read: (rows: Row[]) => Values) {
  return deltas(samples, rows, read, (delta, seconds) =>
    Object.fromEntries(
      Object.entries(delta).map(([key, value]) => [key, value == null ? null : value / seconds]),
    ),
  )
}

// The last non-null value of a series, for tiles and summaries.
export function lastValue(data: ChartSample[], key: string): number | null {
  for (let index = data.length - 1; index >= 0; index -= 1) {
    const value = data[index].values[key]
    if (typeof value === 'number' && Number.isFinite(value)) return value
  }
  return null
}

// Hours and minutes, in UTC like every time in the console. The exact-values
// table keeps full timestamps.
export const formatTime = (value: number) => new Date(value).toISOString().slice(11, 16)
