import type { ButtonHTMLAttributes } from 'react'

import { buttonClassName, type ButtonVariant } from './buttonClasses'

export type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: ButtonVariant
}

/**
 * Small, theme-token-backed action primitive for controls that should never
 * fall back to the browser's default grey button chrome.
 *
 * It intentionally stays dependency-free so the existing Vite/Tauri renderer
 * can adopt shadcn-style composition incrementally without a Tailwind rewrite.
 */
export function Button({
  variant = 'secondary',
  className,
  type = 'button',
  ...props
}: ButtonProps) {
  return <button {...props} type={type} className={buttonClassName(variant, className)} />
}
