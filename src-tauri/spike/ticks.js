/*
 * DESK-2 timer watch. Injected by src-tauri/src/probe.rs when ORBIT_DESKTOP_TICKS=1 (debug builds
 * only). The shell minimises the window a few seconds later; this counter says whether the WebView
 * keeps running timers while the user cannot see it, which is what breaks voice in a hidden window.
 *
 * A healthy WebView ticks about once a second. A throttled one reports the same count again and
 * again, or nothing at all.
 */
;(() => {
  const started = Date.now()
  let ticks = 0

  const report = () => {
    const internals = window.__TAURI_INTERNALS__
    if (!internals || typeof internals.invoke !== 'function') {
      return
    }
    internals
      .invoke('report_spike', {
        report: {
          kind: 'ticks',
          ticks,
          hidden: document.hidden,
          visibility: document.visibilityState,
          elapsedMs: Date.now() - started,
        },
      })
      .catch(() => {})
  }

  setInterval(() => {
    ticks += 1
    if (ticks % 5 === 0) {
      report()
    }
  }, 1000)

  // One report while the window is still visible, as the baseline for the comparison.
  report()
})()
