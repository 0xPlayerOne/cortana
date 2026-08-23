export type ButtonVariant = 'primary' | 'secondary' | 'compact' | 'ghost' | 'icon' | 'danger'

export function buttonClassName(variant: ButtonVariant = 'secondary', className?: string) {
  return ['cortana-button', `cortana-button--${variant}`, className].filter(Boolean).join(' ')
}
