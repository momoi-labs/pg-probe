import { TimeRangeControl, type ChartSample, type TimeRange } from '@momoi-labs/kiso-react'
import { createContext, useContext, useState, type ReactNode } from 'react'
import type { Sample } from './api.ts'

type RangeState = { range: TimeRange | null; setRange: (range: TimeRange) => void }

// The interval every chart shows. Null is the entire run.
const RangeContext = createContext<RangeState>({ range: null, setRange: () => {} })

export const useRange = () => useContext(RangeContext)

// The run's first and last sample, or null when they cannot make a range.
export function runBounds(samples: Sample[] | undefined): TimeRange | null {
  if (!samples?.length) return null
  const from = Date.parse(samples[0].taken_at_utc)
  const to = Date.parse(samples[samples.length - 1].taken_at_utc)
  return from < to ? { from, to } : null
}

// Holds the range for one artifact. Opening another artifact shows its
// entire run again.
export function RangeProvider({ artifact, children }: { artifact: string; children: ReactNode }) {
  const [state, setState] = useState<{ artifact: string; range: TimeRange } | null>(null)
  const range = state?.artifact === artifact ? state.range : null
  const setRange = (next: TimeRange) => setState({ artifact, range: next })
  return <RangeContext value={{ range, setRange }}>{children}</RangeContext>
}

export function RangePicker({ bounds }: { bounds: TimeRange }) {
  const { range, setRange } = useRange()
  return (
    <TimeRangeControl
      label="Time range"
      bounds={bounds}
      value={range ?? bounds}
      onValueChange={setRange}
    />
  )
}

// The samples inside the range. Rates keep the value computed from the
// sample before the range.
export function inRange(data: ChartSample[], range: TimeRange | null): ChartSample[] {
  if (!range) return data
  return data.filter((point) => point.timestamp >= range.from && point.timestamp <= range.to)
}
