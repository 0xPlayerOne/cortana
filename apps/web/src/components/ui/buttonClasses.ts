export type ButtonVariant = 'primary' | 'secondary' | 'compact' | 'ghost' | 'icon'

export function buttonClassName(variant: ButtonVariant = 'secondary', className?: string) {
  return ['cortana-button', `cortana-button--${variant}`, className].filter(Boolean).join(' ')
}
