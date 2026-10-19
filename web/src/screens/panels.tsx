import {
  Badge,
  Card,
  Chart,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  DashboardPanel,
  Skeleton,
  type ChartSample,
  type ChartSeries,
} from '@momoi-labs/kiso-react'
import type { ReactNode } from 'react'
import { formatTime } from '../series.ts'

type Span = 3 | 4 | 6 | 8 | 12

type PanelProps = {
  title: string
  sub?: string
  span: Span
  badge?: string
  loading?: boolean
  children: ReactNode
}

// Anchor for a panel, from its title: "CPU utilization" -> "cpu-utilization".
// Overview tiles link to it.
export const panelId = (title: string) => title.toLowerCase().replace(/[^a-z0-9]+/g, '-')

export function Panel({ title, sub, span, badge, loading, children }: PanelProps) {
  return (
    <DashboardPanel span={span} id={panelId(title)} className="panel-anchor">
      <Card>
        <CardHeader>
          <CardTitle>{title}</CardTitle>
          {sub && <CardDescription>{sub}</CardDescription>}
          {badge && (
            <CardAction>
              <Badge variant="neutral">{badge}</Badge>
            </CardAction>
          )}
        </CardHeader>
        <CardContent>{loading ? <Skeleton variant="block" /> : children}</CardContent>
      </Card>
    </DashboardPanel>
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

// Text that explains a value: its formula or why it is missing.
export function Note({ children }: { children: ReactNode }) {
  return <p className="muted">{children}</p>
}

type ChartPanelProps = {
  // Names the panel's anchor, which Overview tiles link to.
  title: string
  span: Span
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
  note?: ReactNode
}

// A time series panel. The chart caption is the panel's title, so the card
// has no header of its own. Every chart shares one sync id, so inspecting a
// sample in one panel inspects the same time in the others.
export function ChartPanel({
  title,
  span,
  loading,
  label,
  data,
  series,
  variant,
  min,
  max,
  formatValue,
  note,
}: ChartPanelProps) {
  // Kiso charts take one to five series. None means the artifact holds no
  // data for this panel, for example PostgreSQL panels in a Linux capture.
  if (!loading && series.length === 0) {
    return (
      <Panel title={label} span={span}>
        <Note>Not recorded in this artifact.</Note>
      </Panel>
    )
  }
  return (
    <DashboardPanel span={span} id={panelId(title)} className="panel-anchor">
      <Card>
        <CardContent>
          {loading ? (
            <Skeleton variant="block" />
          ) : (
            <Chart
              label={label}
              data={data}
              series={series}
              variant={variant}
              legend="inline"
              min={min}
              max={max}
              syncId="session"
              formatTime={formatTime}
              formatValue={formatValue}
            />
          )}
          {note && <Note>{note}</Note>}
        </CardContent>
      </Card>
    </DashboardPanel>
  )
}
