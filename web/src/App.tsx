import {
  EmptyState,
  EmptyStateDescription,
  EmptyStateTitle,
} from '@momoi-labs/kiso-react'

export default function App() {
  return (
    <main className="console">
      <EmptyState variant="informational">
        <EmptyStateTitle asChild>
          <h1>pg-probe console</h1>
        </EmptyStateTitle>
        <EmptyStateDescription>
          The review screens are not built yet.
        </EmptyStateDescription>
      </EmptyState>
    </main>
  )
}
