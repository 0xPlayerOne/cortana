import { createContext, type ReactNode, useContext } from 'react'

import type { Badge } from '@/components/shadcn/badge'
import type { Alert, AlertDescription, AlertTitle } from '@/components/shadcn/alert'
import type { Button } from '@/components/shadcn/button'
import type { Card } from '@/components/shadcn/card'
import type { Checkbox } from '@/components/shadcn/checkbox'
import type {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/shadcn/empty'
import type { Input } from '@/components/shadcn/input'
import type { Progress } from '@/components/shadcn/progress'
import type { ScrollArea } from '@/components/shadcn/scroll-area'
import type {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/shadcn/select'
import type { Tabs, TabsList, TabsTrigger } from '@/components/shadcn/tabs'
import type { Skeleton } from '@/components/shadcn/skeleton'
import type { Spinner } from '@/components/shadcn/spinner'
import type { Switch } from '@/components/shadcn/switch'
import type { Textarea } from '@/components/shadcn/textarea'
import type { Toggle } from '@/components/shadcn/toggle'

export type M7SurfacePrimitives = {
  Alert: typeof Alert
  AlertDescription: typeof AlertDescription
  AlertTitle: typeof AlertTitle
  Badge: typeof Badge
  Button: typeof Button
  Card: typeof Card
  Checkbox: typeof Checkbox
  Empty: typeof Empty
  EmptyContent: typeof EmptyContent
  EmptyDescription: typeof EmptyDescription
  EmptyHeader: typeof EmptyHeader
  EmptyMedia: typeof EmptyMedia
  EmptyTitle: typeof EmptyTitle
  Input: typeof Input
  Progress: typeof Progress
  ScrollArea: typeof ScrollArea
  Select: typeof Select
  SelectContent: typeof SelectContent
  SelectItem: typeof SelectItem
  SelectTrigger: typeof SelectTrigger
  SelectValue: typeof SelectValue
  Tabs: typeof Tabs
  TabsList: typeof TabsList
  TabsTrigger: typeof TabsTrigger
  Skeleton: typeof Skeleton
  Spinner: typeof Spinner
  Switch: typeof Switch
  Textarea: typeof Textarea
  Toggle: typeof Toggle
}

const M7SurfacePrimitivesContext = createContext<M7SurfacePrimitives | null>(null)

export function M7SurfacePrimitivesProvider({
  value,
  children,
}: {
  value: M7SurfacePrimitives
  children: ReactNode
}) {
  return (
    <M7SurfacePrimitivesContext.Provider value={value}>
      {children}
    </M7SurfacePrimitivesContext.Provider>
  )
}

// The hook and provider must share this module-scoped context; only the
// shadcn renderer supplies the runtime component table.
// eslint-disable-next-line react-refresh/only-export-components
export function useM7SurfacePrimitives() {
  return useContext(M7SurfacePrimitivesContext)
}
