export async function writeClipboardText(value: string): Promise<void> {
  if (typeof navigator.clipboard?.writeText === 'function') {
    await navigator.clipboard.writeText(value)
    return
  }

  // Older or restricted WebViews may omit the async Clipboard API. Keep the
  // fallback local to this renderer and fail visibly if the platform also
  // rejects the legacy copy command.
  const textarea = document.createElement('textarea')
  textarea.value = value
  textarea.setAttribute('readonly', '')
  textarea.style.position = 'fixed'
  textarea.style.opacity = '0'
  document.body.appendChild(textarea)
  textarea.select()
  let copied = false
  try {
    copied =
      typeof document.execCommand === 'function' && document.execCommand.call(document, 'copy')
  } finally {
    textarea.remove()
  }
  if (!copied) throw new Error('Clipboard is unavailable in this WebView')
}
