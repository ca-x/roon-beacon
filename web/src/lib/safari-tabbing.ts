// Safari's Tab key skips buttons, links and non-text form controls (range,
// checkbox, color, select) unless the user turns on
// "Press Tab to highlight each item on a webpage" (Option-Tab reaches them
// either way). It does stop on anything with an explicit tabindex, so in
// Safari each of those, and every summary, that doesn't set one gets
// tabindex="0", including those rendered later. Anything that already sets
// a tabindex is left alone, so components that manage focus themselves
// (radio groups, sliders, menus with roving focus) keep their own order.
// Native radios are left out: a tab stop on each would break the browser's
// one stop per group. Other browsers already tab to these and are untouched.

const selector = [
  "button",
  "a[href]",
  "summary",
  'input[type="range"]',
  'input[type="checkbox"]',
  'input[type="color"]',
  "select",
]
  .map((s) => `${s}:not([tabindex])`)
  .join(", ")

function isSafari() {
  const ua = navigator.userAgent
  return /Safari\//.test(ua) && !/Chrome\/|Chromium\/|CriOS\/|FxiOS\/|EdgiOS\/|Android/.test(ua)
}

function mark(root: ParentNode) {
  if (root instanceof Element && root.matches(selector)) root.setAttribute("tabindex", "0")
  for (const el of root.querySelectorAll(selector)) el.setAttribute("tabindex", "0")
}

export function installSafariTabbing() {
  if (typeof document === "undefined" || !isSafari()) return
  mark(document)
  new MutationObserver((records) => {
    for (const record of records) {
      for (const node of record.addedNodes) if (node instanceof Element) mark(node)
    }
  }).observe(document.body, { childList: true, subtree: true })
}
