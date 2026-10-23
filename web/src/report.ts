import { useMatches } from '@tanstack/react-router'

// True on the printed report, where nothing can be selected or opened later.
export const useInReport = () =>
  useMatches({ select: (matches) => matches.some((match) => match.routeId === '/$snapshotId/report') })
