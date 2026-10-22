import {
  Badge,
  Chart,
  EmptyState,
  EmptyStateDescription,
  EmptyStateTitle,
  DashboardGrid,
  DashboardPanel,
  Skeleton,
  type ChartSample,
  type ChartSeries,
  type ChartSummaryStat,
  type PaneGridLayout,
} from '@momoi-labs/kiso-react'
import type { ReactNode } from 'react'
import { formatTime } from '../series.ts'

// One outlier sets the max and moves the average, so the legend table
// shows the normal range as percentiles and the last sample.
const legendSummary: ChartSummaryStat[] = ['p50', 'p75', 'p95', 'p99', 'current']

type PanelProps = {
  title: string
  sub?: string
  badge?: string
  loading?: boolean
  children: ReactNode
}

// Anchor for a panel, from its title: "CPU utilization" -> "cpu-utilization".
// Overview tiles link to it.
export const panelId = (title: string) => title.toLowerCase().replace(/[^a-z0-9]+/g, '-')

// A section's panels, which the reader can move and resize. The layout is
// saved in the browser per section and restored on the next visit.
export function PanelGrid({ id, label, children }: { id: string; label: string; children: ReactNode }) {
  const key = `pg-probe.layout.${id}`
  let saved: PaneGridLayout | undefined
  try {
    saved = JSON.parse(localStorage.getItem(key) ?? 'null') ?? undefined
  } catch {
    saved = undefined
  }
  return (
    <DashboardGrid
      aria-label={label}
      defaultLayout={saved}
      onLayoutChange={(layout) => localStorage.setItem(key, JSON.stringify(layout))}
    >
      {children}
    </DashboardGrid>
  )
}

// A place in a PanelGrid. DashboardGrid reads the pane id and default width, in
// columns out of twelve, from its direct children, so the section sets them
// here and the panel inside renders the pane.
export function Slot({ children }: { id: string; size: 3 | 4 | 6 | 8 | 12; children: ReactNode }) {
  return children
}

// A pane of a PanelGrid. The id comes from the panel title and matches its
// Slot. The heading may say more, such as a chart's measure and unit. The
// anchor inside is the target of Overview tile links, since a pane takes no
// DOM id of its own.
function Pane({
  title,
  heading = title,
  badge,
  children,
}: {
  title: string
  heading?: string
  badge?: string
  children: ReactNode
}) {
  return (
    <DashboardPanel
      id={panelId(title)}
      title={heading}
      actions={badge && <Badge variant="neutral">{badge}</Badge>}
    >
      <span id={panelId(title)} className="panel-anchor" />
      {children}
    </DashboardPanel>
  )
}

export function Panel({ title, sub, badge, loading, children }: PanelProps) {
  return (
    <Pane title={title} badge={badge}>
      {sub && <p className="muted">{sub}</p>}
      {loading ? <Skeleton variant="block" /> : children}
    </Pane>
  )
}

// A panel the artifact cannot draw yet. The badge names the missing work, as
// in the collection plan: a second sample, a new collector or a schema change.
export function GapPanel({
  gap,
  children,
  ...panel
}: Omit<PanelProps, 'badge' | 'loading'> & { gap: string }) {
  return (
    <Panel {...panel} badge={gap}>
      <p className="muted">{children}</p>
    </Panel>
  )
}

// Text that explains why a value is missing. It stays in view.
export function Note({ children }: { children: ReactNode }) {
  return <p className="muted">{children}</p>
}

// A panel whose metric the observed server or host cannot provide, such as a
// PostgreSQL view added in a later release. It is a known result, so it uses
// an informational empty state, not a note or an error.
export function NotSupportedPanel({
  title,
  label,
  on,
  children,
}: {
  title: string
  // The heading the chart would have, with its measure and unit.
  label: string
  // Where it is not supported, for example "PostgreSQL 9.3" or "this host".
  on: string
  children: ReactNode
}) {
  return (
    <Pane title={title} heading={label}>
      <EmptyState variant="informational" size="sm">
        <EmptyStateTitle>Not supported on {on}</EmptyStateTitle>
        <EmptyStateDescription>{children}</EmptyStateDescription>
      </EmptyState>
    </Pane>
  )
}

type ChartPanelProps = {
  // Names the panel's anchor, which Overview tiles link to.
  title: string
  loading?: boolean
  // The one visible title: what the chart shows and its unit, for example
  // "CPU utilization by state (%)". Axis values carry no unit.
  label: string
  data: ChartSample[]
  series: ChartSeries[]
  variant?: 'line' | 'stacked-area'
  min?: number
  max?: number
  formatValue?: (value: number) => string
}

// A time series panel. The pane heading is the chart label, with the measure
// and its unit, and the chart hides its own caption inside a pane. Every chart
// shares one sync id: hover marks the same time in the others, and a click
// pins it there.
export function ChartPanel({
  title,
  loading,
  label,
  data,
  series,
  variant,
  min,
  max,
  formatValue,
}: ChartPanelProps) {
  // Kiso charts take one to five series. None means the artifact holds no
  // data for this panel, for example PostgreSQL panels in a Linux capture.
  if (!loading && series.length === 0) {
    return (
      <Pane title={title} heading={label}>
        <Note>Not recorded in this artifact.</Note>
      </Pane>
    )
  }
  return (
    <Pane title={title} heading={label}>
      {loading ? (
        <Skeleton variant="block" />
      ) : (
        <Chart
          label={label}
          data={data}
          series={series}
          variant={variant}
          min={min}
          max={max}
          summary={legendSummary}
          syncId="session"
          formatTime={formatTime}
          formatValue={formatValue}
        />
      )}
    </Pane>
  )
}
