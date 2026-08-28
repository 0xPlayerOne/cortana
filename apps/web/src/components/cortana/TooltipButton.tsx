import type { ComponentProps } from 'react'

import { Button } from '../shadcn/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '../shadcn/tooltip'

type TooltipButtonProps = ComponentProps<typeof Button> & {
  tooltip?: string
  tooltipSide?: 'top' | 'right' | 'bottom' | 'left'
}

/** Shared shadcn button composition for concise, accessible action help. */
export function TooltipButton({ tooltip, tooltipSide, ...props }: TooltipButtonProps) {
  if (!tooltip) return <Button {...props} />
  return (
    <Tooltip>
      <TooltipTrigger render={<Button {...props} />} />
      <TooltipContent side={tooltipSide}>{tooltip}</TooltipContent>
    </Tooltip>
  )
}
