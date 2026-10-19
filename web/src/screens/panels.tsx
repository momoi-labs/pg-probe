import {
  Badge,
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  DashboardPanel,
  Skeleton,
} from '@momoi-labs/kiso-react'
import type { ReactNode } from 'react'

type Span = 3 | 4 | 6 | 8 | 12

type PanelProps = {
  title: string
  sub?: string
  span: Span
  badge?: string
  loading?: boolean
  children: ReactNode
}

export function Panel({ title, sub, span, badge, loading, children }: PanelProps) {
  return (
    <DashboardPanel span={span}>
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
