import { Check, LoaderCircle, X } from 'lucide-react'

export function StatusGlyph({
  passed,
  optional = false,
  pending = false,
}: {
  passed: boolean
  optional?: boolean
  pending?: boolean
}) {
  return (
    <i
      className={`status-glyph ${pending ? 'pending' : passed ? 'passed' : optional ? 'optional' : 'failed'}`}
      aria-label={pending ? 'In progress' : passed ? 'Passed' : optional ? 'Optional' : 'Failed'}
      role="img"
    >
      {pending ? (
        <LoaderCircle className="spin" size={13} />
      ) : passed ? (
        <Check size={13} />
      ) : (
        <X size={13} />
      )}
    </i>
  )
}
