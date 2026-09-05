export async function waitForInitialKeyboardTarget(page) {
  const skipLink = page.locator('a[href="#main-content"]')
  await skipLink.waitFor({ state: 'visible' })
  return skipLink
}

export async function setViewportAndWaitForLayout(page, viewport) {
  await page.setViewportSize(viewport)
  await page.evaluate(
    () =>
      new Promise((resolveLayout) =>
        requestAnimationFrame(() => requestAnimationFrame(resolveLayout))
      )
  )
}
