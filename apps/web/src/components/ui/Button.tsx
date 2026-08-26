import type { ButtonHTMLAttributes } from 'react'

import { buttonClassName, type ButtonVariant } from './buttonClasses'

export type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: ButtonVariant
}

/**
 * Temporary legacy action primitive. M7 removes this file after every caller
 * moves to the generated shadcn Button.
 */
export function Button({
  variant = 'secondary',
  className,
  type = 'button',
  ...props
}: ButtonProps) {
  return <button {...props} type={type} className={buttonClassName(variant, className)} />
}
