import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { RouterProvider } from '@tanstack/react-router'
import '@momoi-labs/kiso-react/styles.css'
import './app.css'
import { createAppRouter } from './router.tsx'

// Artifacts never change while the console reads them.
const queryClient = new QueryClient({
  defaultOptions: { queries: { staleTime: Infinity, retry: false } },
})
const router = createAppRouter(queryClient)

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  </StrictMode>,
)
