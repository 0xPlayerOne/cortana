import { Check, X } from 'lucide-react'

export function StatusGlyph({ passed, optional = false }: { passed: boolean; optional?: boolean }) {
  return (
    <i className={`status-glyph ${passed ? 'passed' : optional ? 'optional' : 'failed'}`}>
      {passed ? <Check size={13} /> : <X size={13} />}
    </i>
  )
}
