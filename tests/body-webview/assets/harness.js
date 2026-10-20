// This asset is embedded only by the body-webview-test build configuration.
// No application frontend imports it. postMessage never authorizes a link.
(() => {
  const config = window.__MAILX_BODY_TEST_CONFIG;
  if (!config || !config.nonce || !config.documents) {
    document.querySelector('#status').textContent = '测试构建缺少合成样本配置';
    return;
  }
  let frame = document.querySelector('#body');
  let viewGeneration = 0;
  const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
  const evidence = {
    ready: false,
    selected: null,
    emailId: null,
    documentToken: null,
    viewGeneration: 0,
    bodyLinksLoaded: false,
    prepareError: null,
    frameLoads: 0,
    observations: [],
    links: [],
    acceptedLinks: [],
    currentLinkRequest: null,
    requests: [],
    numberedLinks: [],
    numberedRequestSucceeded: false,
    numberedInput: null,
    numberedError: null,
    unexpectedMessages: 0,
    parentSecret: 'synthetic-parent-state',
  };
  window.__MAILX_BODY_TEST_EVIDENCE = evidence;
  document.addEventListener('mousedown', (event) => {
    evidence.nativePointer = { target: event.target.tagName, x: event.clientX, y: event.clientY, trusted: event.isTrusted };
  }, true);
  window.__MAILX_TEST_SECRET = 'synthetic-parent-state';
  function display() {
    document.querySelector('#evidence').textContent = JSON.stringify({
      selected: evidence.selected,
      frameLoads: evidence.frameLoads,
      observations: evidence.observations.length,
      linkEvents: evidence.links.length,
      protocolRequests: evidence.requests.length,
    });
  }
  window.__MAILX_BODY_TEST_RECORD_REQUEST = (record) => {
    evidence.requests.push(record);
    display();
  };
  window.addEventListener('message', (event) => {
    if (event.source !== frame.contentWindow) return;
    evidence.unexpectedMessages += 1;
    display();
  });
  async function prepareFrame(name, generation, emailId, nextFrame) {
    const current = () => viewGeneration === generation;
    try {
      const prepared = await window.__TAURI__.core.invoke('emails_prepare_body', { emailId });
      if (!prepared.ok) throw new Error(prepared.error.code);
      if (!current()) return;
      const documentToken = prepared.data.documentToken;
      if (!uuid.test(documentToken)) throw new Error('Invalid prepared document token');
      // convertFileSrc encodes its entire argument. Ask it for the platform
      // origin, then construct route/query components with the URL API.
      const url = new URL(window.__TAURI__.core.convertFileSrc('', 'mailx-body'));
      url.pathname = `/document/${emailId}`;
      url.searchParams.set('token', documentToken);
      const loadedUrl = url.href;
      const bound = () => current() && frame === nextFrame && evidence.documentToken === documentToken;
      nextFrame.addEventListener('load', async () => {
        if (!bound() || nextFrame.src !== loadedUrl) return;
        evidence.frameLoads += 1;
        display();
        try {
          const response = await window.__TAURI__.core.invoke('emails_body_links', { emailId, documentToken });
          if (!response.ok) throw new Error(response.error.code);
          if (!bound()) return;
          const { documentToken: listedToken, links } = response.data;
          if (listedToken !== documentToken) throw new Error('Link metadata changed document binding');
          if (!config.numberedLinks && links.length) throw new Error('Unexpected numbered-link fallback');
          evidence.numberedLinks = links;
          evidence.bodyLinksLoaded = true;
          const container = document.querySelector('#numbered-links');
          container.replaceChildren();
          for (const link of links) {
            const button = document.createElement('button');
            button.textContent = `Link ${link.index + 1}: ${link.url}`;
            button.addEventListener('click', async (event) => {
              if (!bound()) return;
              evidence.numberedInput = { trusted: event.isTrusted, x: event.clientX, y: event.clientY };
              // Real main-window command and real event; no system opener is called.
              try {
                const result = await window.__TAURI__.core.invoke('links_request', { emailId, index: link.index, documentToken });
                if (!bound()) return;
                evidence.numberedRequestSucceeded = result.ok;
                if (!result.ok) evidence.numberedError = result.error.code;
              } catch (error) {
                if (bound()) evidence.numberedError = error.message;
              }
            });
            container.append(button);
          }
          display();
        } catch (error) {
          if (bound()) evidence.numberedError = error.message;
        }
      });
      evidence.documentToken = documentToken;
      // A fresh element makes a previous view's late load callback distinguishable.
      nextFrame.src = loadedUrl;
      frame.replaceWith(nextFrame);
      frame = nextFrame;
      document.querySelector('#status').textContent = `测试样本：${name}`;
    } catch (error) {
      if (current()) evidence.prepareError = error.message;
    }
  }
  window.__MAILX_BODY_TEST = {
    load(name) {
      const url = config.documents[name];
      if (!url) throw new Error('Unknown synthetic fixture');
      const emailId = new URL(url).pathname.split('/').at(-1);
      if (!uuid.test(emailId)) throw new Error('Invalid synthetic fixture identity');
      const generation = ++viewGeneration;
      evidence.selected = name;
      evidence.emailId = emailId;
      evidence.documentToken = null;
      evidence.viewGeneration = generation;
      evidence.bodyLinksLoaded = false;
      evidence.prepareError = null;
      evidence.currentLinkRequest = null;
      evidence.numberedLinks = [];
      evidence.numberedError = null;
      evidence.numberedRequestSucceeded = false;
      document.querySelector('#numbered-links').replaceChildren();
      const nextFrame = frame.cloneNode(false);
      nextFrame.removeAttribute('src');
      frame.removeAttribute('src');
      document.querySelector('#status').textContent = `正在准备测试样本：${name}`;
      // Keep the driver's synchronous entry point; runners poll the bound state.
      void prepareFrame(name, generation, emailId, nextFrame);
      return generation;
    },
    state() {
      let parentCanReadFrame = false;
      let parentCanReadFrameInternals = false;
      const parentAccessErrors = {};
      try { parentCanReadFrame = Boolean(frame.contentWindow.document.body); } catch (error) { parentAccessErrors.frameDom = error.name; }
      try { parentCanReadFrameInternals = Boolean(frame.contentWindow.__TAURI_INTERNALS__); } catch (error) { parentAccessErrors.frameInternals = error.name; }
      const rectangle = frame.getBoundingClientRect();
      const button = document.querySelector('#numbered-links button')?.getBoundingClientRect();
      return {
        ...evidence,
        parentSecret: window.__MAILX_TEST_SECRET,
        userAgent: navigator.userAgent,
        appOrigin: location.origin,
        appUrl: location.href,
        frameUrl: frame.src,
        sandbox: frame.getAttribute('sandbox'),
        referrerPolicy: frame.referrerPolicy,
        parentCanReadFrame,
        parentCanReadFrameInternals,
        parentAccessErrors,
        viewport: { width: innerWidth, height: innerHeight },
        numberedLinkRectangle: button ? { x: button.x, y: button.y, width: button.width, height: button.height } : null,
        frameRectangle: { x: rectangle.x, y: rectangle.y, width: rectangle.width, height: rectangle.height },
        screen: { x: window.screenX, y: window.screenY, pixelRatio: window.devicePixelRatio },
      };
    },
  };
  // This is the real Tauri event surface. A forged postMessage never enters it.
  window.__TAURI__.event.listen('mailx://link-request', (event) => {
    const request = event.payload;
    // Preserve every real event for zero-forgery assertions, even when rejected.
    evidence.links.push(request);
    if (request.emailId === evidence.emailId && request.documentToken === evidence.documentToken &&
      uuid.test(request.documentToken) && uuid.test(request.requestId) &&
      Number.isSafeInteger(request.index) && request.index >= 0 && typeof request.url === 'string') {
      evidence.acceptedLinks.push(request);
      evidence.currentLinkRequest = { ...request, viewGeneration };
    }
    display();
  }).then(() => {
    evidence.ready = true;
    document.querySelector('#status').textContent = '合成样本入口已就绪';
  }).catch(() => {
    document.querySelector('#status').textContent = '真实 Tauri 链接事件订阅失败';
  });
})();
