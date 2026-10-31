/*
 * DESK-1 spike probe. Injected into the loaded page by src-tauri/src/probe.rs, debug builds
 * only, and only when ORBIT_DESKTOP_PROBE=1. It asks the WebView what orbit-web relies on and
 * prints one JSON line on stdout, so the "работает / нет" table is measured, not guessed.
 *
 * Nothing here changes the page. Every check is wrapped: a WebView that throws on
 * `new AudioContext()` is an answer too.
 */
;(async () => {
  const checks = []

  const check = async (id, run) => {
    try {
      const result = await run()
      if (Array.isArray(result)) {
        checks.push({ id, ok: Boolean(result[0]), detail: result[1] })
      } else {
        checks.push({ id, ok: Boolean(result), detail: undefined })
      }
    } catch (error) {
      checks.push({ id, ok: false, detail: String(error) })
    }
  }

  const describe = (value) => (value === undefined ? 'undefined' : String(value))

  // --- what the shell itself provides -------------------------------------
  await check('shell.ipc', () => {
    const internals = window.__TAURI_INTERNALS__
    return [Boolean(internals && typeof internals.invoke === 'function'), typeof internals]
  })

  await check('secure-context', () => [isSecureContext, location.origin])

  // --- voice: microphone, WebRTC, audio graph -----------------------------
  await check('media.getUserMedia', () => {
    const devices = navigator.mediaDevices
    return [typeof devices?.getUserMedia === 'function', typeof devices?.getUserMedia]
  })

  await check('media.enumerateDevices', () => {
    const devices = navigator.mediaDevices
    return [typeof devices?.enumerateDevices === 'function', typeof devices?.enumerateDevices]
  })

  await check('media.supportedConstraints', () => {
    const constraints = navigator.mediaDevices?.getSupportedConstraints?.() ?? {}
    const keys = Object.keys(constraints)
    return [
      keys.length > 0,
      {
        keys: keys.length,
        echoCancellation: constraints.echoCancellation,
        noiseSuppression: constraints.noiseSuppression,
        autoGainControl: constraints.autoGainControl,
        sampleRate: constraints.sampleRate,
      },
    ]
  })

  await check('rtc.peerConnection', () => {
    const Peer = window.RTCPeerConnection
    if (!Peer) {
      return [false, 'no RTCPeerConnection']
    }
    const prototype = Peer.prototype
    return [
      typeof prototype.createOffer === 'function' && typeof prototype.addTransceiver === 'function',
      {
        addTransceiver: typeof prototype.addTransceiver,
        setCodecPreferences: typeof prototype.setCodecPreferences,
        getStats: typeof prototype.getStats,
      },
    ]
  })

  await check('rtc.codecs', () => {
    const of = typeof window.RTCRtpSender?.getCapabilities === 'function'
    if (!of) {
      return [false, 'no RTCRtpSender.getCapabilities']
    }
    const audio = window.RTCRtpSender.getCapabilities('audio')?.codecs ?? []
    const video = window.RTCRtpSender.getCapabilities('video')?.codecs ?? []
    return [
      audio.some((codec) => codec.mimeType === 'audio/opus') &&
        video.some((codec) => codec.mimeType === 'video/VP8'),
      {
        audio: audio.map((codec) => codec.mimeType),
        video: video.map((codec) => codec.mimeType),
      },
    ]
  })

  await check('rtc.dataChannel', () => typeof window.RTCDataChannel === 'function')

  await check('audio.context', async () => {
    const Context = window.AudioContext ?? window.webkitAudioContext
    if (!Context) {
      return [false, 'no AudioContext']
    }
    const context = new Context({ sampleRate: 48000 })
    const detail = {
      state: context.state,
      sampleRate: context.sampleRate,
      audioWorklet: Boolean(context.audioWorklet),
      audioWorkletNode: typeof window.AudioWorkletNode === 'function',
    }
    await context.close()
    return [detail.audioWorklet && detail.audioWorkletNode, detail]
  })

  await check('audio.sharedArrayBuffer', () => typeof window.SharedArrayBuffer === 'function')

  await check('media.recorder', () => {
    const Recorder = window.MediaRecorder
    if (!Recorder) {
      return [false, 'no MediaRecorder']
    }
    return [
      Recorder.isTypeSupported('audio/webm;codecs=opus'),
      { webmOpus: Recorder.isTypeSupported('audio/webm;codecs=opus') },
    ]
  })

  // Track plumbing without a capture: proves MediaStream and canvas capture exist, asks for nothing.
  await check('media.stream', () => {
    const Stream = window.MediaStream
    if (!Stream) {
      return [false, 'no MediaStream']
    }
    const stream = new Stream()
    const canvas = document.createElement('canvas')
    return [true, { tracks: stream.getTracks().length, canvasCapture: typeof canvas.captureStream }]
  })

  // --- screen sharing ------------------------------------------------------
  await check('media.getDisplayMedia', () => {
    const devices = navigator.mediaDevices
    return [typeof devices?.getDisplayMedia === 'function', typeof devices?.getDisplayMedia]
  })

  // Safari/WKWebView only lists screens when asked for them; presence here is a hint, not a share.
  await check('media.displaySurface', () => {
    const constraints = navigator.mediaDevices?.getSupportedConstraints?.() ?? {}
    return [Boolean(constraints.displaySurface), { displaySurface: constraints.displaySurface }]
  })

  // --- secret chats and RNNoise: WASM, storage ----------------------------
  await check('wasm.validate', () => {
    // Smallest valid module: magic number + version.
    const bytes = new Uint8Array([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00])
    return [WebAssembly.validate(bytes), { instantiateStreaming: typeof WebAssembly.instantiateStreaming }]
  })

  await check('wasm.instantiate', async () => {
    const bytes = new Uint8Array([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00])
    const module = await WebAssembly.instantiate(bytes)
    return [Boolean(module.instance), 'instantiated']
  })

  await check('storage.indexedDb', async () => {
    const open = indexedDB.open('orbit-desktop-probe', 1)
    const db = await new Promise((resolve, reject) => {
      open.onsuccess = () => resolve(open.result)
      open.onerror = () => reject(open.error)
    })
    db.close()
    indexedDB.deleteDatabase('orbit-desktop-probe')
    return [true, 'opened']
  })

  await check('storage.localStorage', () => {
    const key = 'orbit-desktop-probe'
    localStorage.setItem(key, '1')
    const value = localStorage.getItem(key)
    localStorage.removeItem(key)
    return [value === '1', 'round trip']
  })

  await check('storage.cookies', () => {
    document.cookie = 'orbit-desktop-probe=1; path=/; SameSite=Lax'
    return [navigator.cookieEnabled && document.cookie.includes('orbit-desktop-probe=1'), {
      cookieEnabled: navigator.cookieEnabled,
      cookie: document.cookie.length > 0 ? 'present' : 'empty',
    }]
  })

  // Cookies without `Secure` are refused by WebKit on http, so a dev build needs the check above.
  await check('storage.persist', async () => {
    if (!navigator.storage?.persist) {
      return [false, 'no storage.persist']
    }
    const persisted = await navigator.storage.persist()
    return [true, { persisted }]
  })

  // --- notifications, push, service worker --------------------------------
  await check('notifications', () => {
    const Notification = window.Notification
    return [Boolean(Notification), { api: typeof Notification, permission: Notification?.permission }]
  })

  await check('push', () => {
    const detail = {
      PushManager: Boolean(window.PushManager),
      ServiceWorkerRegistration: Boolean(window.ServiceWorkerRegistration),
    }
    return [detail.PushManager, detail]
  })

  await check('serviceWorker', () => 'serviceWorker' in navigator)

  // --- the shell's plugins: what DESK-3 added for PlatformAdapter ----------
  await check('shell.plugins', async () => {
    const internals = window.__TAURI_INTERNALS__
    if (!internals || typeof internals.invoke !== 'function') {
      return [false, 'no IPC']
    }
    const call = async (command) => {
      try {
        return { ok: true, value: await internals.invoke(command) }
      } catch (error) {
        return { ok: false, error: String(error) }
      }
    }
    // plugin-os answers from constants injected into the page, not over IPC; the app reads
    // the same object, so checking it is checking what PlatformAdapter will see.
    const osInternals = window.__TAURI_OS_PLUGIN_INTERNALS__
    const os = osInternals
      ? {
          ok: true,
          value: {
            platform: osInternals.platform,
            version: osInternals.version,
            arch: osInternals.arch,
          },
        }
      : { ok: false, error: 'no __TAURI_OS_PLUGIN_INTERNALS__' }
    const [notification, autostart, appVersion] = await Promise.all([
      call('plugin:notification|is_permission_granted'),
      call('plugin:autostart|is_enabled'),
      call('plugin:app|version'),
    ])
    return [
      os.ok && notification.ok && autostart.ok && appVersion.ok,
      { os, notification, autostart, appVersion },
    ]
  })

  // --- can the shell actually deliver a notification? (OPT-IN, DESK-4) ----
  if (window.__ORBIT_NOTIFY_TEST__) {
    await check('shell.notify', async () => {
      const internals = window.__TAURI_INTERNALS__
      if (!internals || typeof internals.invoke !== 'function') {
        return [false, 'no IPC']
      }
      try {
        await internals.invoke('plugin:notification|notify', {
          options: {
            id: 99001,
            title: 'Orbit',
            body: 'Проверка уведомлений из оболочки',
          },
        })
        return [true, 'sent']
      } catch (error) {
        return [false, String(error)]
      }
    })
  }

  // --- did the app itself come up? ----------------------------------------
  // The desktop adapter is loaded before the first render; if that import throws, React never
  // mounts and the window stays empty. Waiting a moment separates "not yet" from "never".
  await check('app.rendered', async () => {
    const deadline = Date.now() + 3000
    while (Date.now() < deadline) {
      const rootElement = document.getElementById('root')
      if (rootElement && rootElement.childElementCount > 0) {
        return [true, { children: rootElement.childElementCount }]
      }
      await new Promise((resolve) => setTimeout(resolve, 100))
    }
    return [false, 'the app did not render']
  })

  // --- the environment, for the report ------------------------------------
  const report = {
    origin: location.origin,
    url: location.href,
    userAgent: navigator.userAgent,
    language: navigator.language,
    screen: {
      width: screen.width,
      height: screen.height,
      availWidth: screen.availWidth,
      availHeight: screen.availHeight,
      devicePixelRatio: devicePixelRatio,
    },
    checks,
    failed: checks.filter((entry) => !entry.ok).map((entry) => entry.id),
  }

  // Kept for a devtools session; the JSON line on stdout is the machine-readable copy.
  window.__ORBIT_PROBE__ = report

  const internals = window.__TAURI_INTERNALS__
  if (internals && typeof internals.invoke === 'function') {
    try {
      await internals.invoke('report_spike', { report })
    } catch (error) {
      console.error('orbit probe: report_spike failed', error)
    }
  } else {
    console.info('orbit probe', report)
  }
})()
