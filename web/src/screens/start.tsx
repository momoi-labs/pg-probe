import {
  Alert,
  AlertContent,
  AlertDescription,
  AlertTitle,
  EmptyState,
  EmptyStateDescription,
  EmptyStateTitle,
  PageHeader,
  PageHeaderTitle,
} from '@momoi-labs/kiso-react'
import type { ErrorComponentProps } from '@tanstack/react-router'

export function NoSnapshots() {
  return (
    <>
      <PageHeader>
        <PageHeaderTitle>Snapshots</PageHeaderTitle>
      </PageHeader>
      <EmptyState variant="first-run">
        <EmptyStateTitle>No snapshots in this directory</EmptyStateTitle>
        <EmptyStateDescription>
          Run <code>pg-probe capture</code>, then start <code>pg-probe analyze</code> with{' '}
          <code>--data-directory</code> pointing at the directory that holds the{' '}
          <code>.sqlite.gz</code> files.
        </EmptyStateDescription>
      </EmptyState>
    </>
  )
}

export function SnapshotsError({ error }: ErrorComponentProps) {
  return (
    <>
      <PageHeader>
        <PageHeaderTitle>Snapshots</PageHeaderTitle>
      </PageHeader>
      <Alert variant="error">
        <AlertContent>
          <AlertTitle>Could not list the snapshots</AlertTitle>
          <AlertDescription>
            {error instanceof Error ? error.message : String(error)}. Check that <code>pg-probe analyze</code> is running and can
            read its data directory, then reload the page.
          </AlertDescription>
        </AlertContent>
      </Alert>
    </>
  )
}
