// This asset is embedded only by the body-webview-test build configuration.
// No application frontend imports it. postMessage never authorizes a link.
(() => {
  const config = window.__MAILX_BODY_TEST_CONFIG;
  if (!config || !config.nonce || !config.documents) {
    document.querySelector('#status').textContent = '测试构建缺少合成样本配置';
    return;
  }
  const frame = document.querySelector('#body');
  const evidence = {
    ready: false,
    selected: null,
    frameLoads: 0,
    observations: [],
    links: [],
    requests: [],
    numberedLinks: [],
    numberedRequestSucceeded: false,
    numberedError: null,
    unexpectedMessages: 0,
    parentSecret: 'synthetic-parent-state',
  };
  window.__MAILX_BODY_TEST_EVIDENCE = evidence;
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
  frame.addEventListener('load', async () => {
    evidence.frameLoads += 1;
    display();
    if (!config.numberedLinks) return;
    const loadedUrl = frame.src;
    const emailId = loadedUrl.split('/').at(-1);
    try {
      const response = await window.__TAURI__.core.invoke('emails_body_links', { emailId });
      if (!response.ok) throw new Error(response.error.code);
      if (frame.src !== loadedUrl) return;
      const { documentToken, links } = response.data;
      evidence.numberedLinks = links;
      const container = document.querySelector('#numbered-links');
      container.replaceChildren();
      for (const link of links) {
        const button = document.createElement('button');
        button.textContent = `Link ${link.index + 1}: ${link.url}`;
        button.addEventListener('click', async () => {
          // Real main-window command and real event; no system opener is called.
          const result = await window.__TAURI__.core.invoke('links_request', { emailId, index: link.index, documentToken });
          evidence.numberedRequestSucceeded = result.ok;
          if (!result.ok) evidence.numberedError = result.error.code;
        });
        container.append(button);
      }
    } catch (error) { evidence.numberedError = error.message; }
  });
  window.__MAILX_BODY_TEST = {
    load(name) {
      const url = config.documents[name];
      if (!url) throw new Error('Unknown synthetic fixture');
      evidence.selected = name;
      evidence.numberedLinks = [];
      evidence.numberedError = null;
      evidence.numberedRequestSucceeded = false;
      document.querySelector('#numbered-links').replaceChildren();
      frame.src = url;
      document.querySelector('#status').textContent = `测试样本：${name}`;
      return url;
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
    evidence.links.push(event.payload);
    display();
  }).then(() => {
    evidence.ready = true;
    document.querySelector('#status').textContent = '合成样本入口已就绪';
  }).catch(() => {
    document.querySelector('#status').textContent = '真实 Tauri 链接事件订阅失败';
  });
})();
