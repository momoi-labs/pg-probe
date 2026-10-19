import {
  ApplicationShell,
  BrandMark,
  Navigation,
  NavigationItem,
  NavigationLink,
  NavigationList,
} from '@momoi-labs/kiso-react'
import { Link, Outlet, useMatches, useParams } from '@tanstack/react-router'

const sections = [
  { to: '/$snapshotId', routeId: '/$snapshotId/', label: 'Metrics' },
  { to: '/$snapshotId/logs', routeId: '/$snapshotId/logs', label: 'Logs' },
  { to: '/$snapshotId/queries', routeId: '/$snapshotId/queries', label: 'Queries' },
] as const

function SectionNavigation() {
  const { snapshotId } = useParams({ strict: false })
  const current = useMatches({ select: (matches) => matches.at(-1)?.routeId })
  if (!snapshotId) return null
  return (
    <Navigation aria-label="Snapshot sections">
      <NavigationList className="nav-row">
        {sections.map((section) => (
          <NavigationItem key={section.to}>
            <NavigationLink
              asChild
              active={current === section.routeId}
            >
              <Link to={section.to} params={{ snapshotId }} activeOptions={{ exact: true }}>
                {section.label}
              </Link>
            </NavigationLink>
          </NavigationItem>
        ))}
      </NavigationList>
    </Navigation>
  )
}

export function Shell() {
  return (
    <ApplicationShell
      layout="topbar"
      brand={
        <Link to="/" className="brand">
          <BrandMark>P</BrandMark>
          pg-probe
        </Link>
      }
      header={<SectionNavigation />}
    >
      <div className="page">
        <Outlet />
      </div>
    </ApplicationShell>
  )
}
