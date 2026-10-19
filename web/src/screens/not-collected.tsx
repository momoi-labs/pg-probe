import { EmptyState, EmptyStateDescription, EmptyStateTitle } from '@momoi-labs/kiso-react'

export function NotCollected({ what }: { what: string }) {
  return (
    <EmptyState variant="informational">
      <EmptyStateTitle>{what} are not collected yet</EmptyStateTitle>
      <EmptyStateDescription>
        Artifacts do not record this today, so there is nothing to show for this snapshot.
      </EmptyStateDescription>
    </EmptyState>
  )
}
