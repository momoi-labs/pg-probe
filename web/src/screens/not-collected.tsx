import { EmptyState, EmptyStateDescription, EmptyStateTitle } from '@momoi-labs/kiso-react'
import { useT } from '../i18n.ts'

export function NotCollected({ what }: { what: string }) {
  const t = useT()
  return (
    <EmptyState variant="informational">
      <EmptyStateTitle>{t('{what} are not collected yet', { what: t(what) })}</EmptyStateTitle>
      <EmptyStateDescription>
        {t('Artifacts do not record this today, so there is nothing to show for this snapshot.')}
      </EmptyStateDescription>
    </EmptyState>
  )
}
