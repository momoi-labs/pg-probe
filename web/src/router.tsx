import type { QueryClient } from '@tanstack/react-query'
import {
  createRootRouteWithContext,
  createRoute,
  createRouter,
  redirect,
} from '@tanstack/react-router'
import { routeId, snapshotsQuery } from './api.ts'
import { Shell } from './screens/shell.tsx'
import { NoSnapshots, SnapshotsError } from './screens/start.tsx'
import { SnapshotLayout } from './screens/snapshot.tsx'
import { Metrics } from './screens/metrics.tsx'
import { Queries } from './screens/queries.tsx'
import { NotCollected } from './screens/not-collected.tsx'
import { Report } from './screens/report.tsx'

const rootRoute = createRootRouteWithContext<{ queryClient: QueryClient }>()({
  component: Shell,
})

// The console opens on the newest artifact. There is no list page: the
// artifact picker in the page header is the way between artifacts.
const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/',
  loader: async ({ context }) => {
    const snapshots = await context.queryClient.ensureQueryData(snapshotsQuery)
    const first = snapshots[0]
    if (first) {
      throw redirect({ to: '/$snapshotId', params: { snapshotId: routeId(first) } })
    }
  },
  component: NoSnapshots,
  errorComponent: SnapshotsError,
})

const snapshotRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '$snapshotId',
  component: SnapshotLayout,
})

const metricsRoute = createRoute({
  getParentRoute: () => snapshotRoute,
  path: '/',
  component: Metrics,
})

const logsRoute = createRoute({
  getParentRoute: () => snapshotRoute,
  path: 'logs',
  component: () => <NotCollected what="Logs" />,
})

const queriesRoute = createRoute({
  getParentRoute: () => snapshotRoute,
  path: 'queries',
  component: Queries,
})

// The whole artifact on one page for printing. ?print=true opens the print
// dialog once it has loaded.
const reportRoute = createRoute({
  getParentRoute: () => snapshotRoute,
  path: 'report',
  validateSearch: (search: Record<string, unknown>): { print?: boolean } =>
    search.print === true || search.print === 'true' ? { print: true } : {},
  component: Report,
})

const routeTree = rootRoute.addChildren([
  indexRoute,
  snapshotRoute.addChildren([metricsRoute, logsRoute, queriesRoute, reportRoute]),
])

export function createAppRouter(queryClient: QueryClient) {
  return createRouter({ routeTree, basepath: '/console', context: { queryClient } })
}

declare module '@tanstack/react-router' {
  interface Register {
    router: ReturnType<typeof createAppRouter>
  }
}
