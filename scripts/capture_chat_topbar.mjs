import fs from 'fs';

async function run() {
  const pages = await fetch('http://127.0.0.1:9333/json').then(r => r.json());
  const page = pages.find(p => p.type === 'page');
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise(r => ws.onopen = r);
  let id = 1;
  function send(method, params = {}) {
    return new Promise((resolve, reject) => {
      const curId = id++;
      const handler = (event) => {
        const msg = JSON.parse(event.data);
        if (msg.id === curId) {
          ws.removeEventListener('message', handler);
          if (msg.error) reject(new Error(msg.error.message));
          else resolve(msg.result);
        }
      };
      ws.addEventListener('message', handler);
      ws.send(JSON.stringify({ id: curId, method, params }));
    });
  }

  // Navigate to chat
  await send('Runtime.evaluate', {
    expression: `
      document.querySelector('.mc-scrim')?.click();
      const back = document.querySelector('button[aria-label*="Voltar"], button[aria-label*="Back"]');
      if (back) back.click();
    `
  });
  await new Promise(r => setTimeout(r, 600));

  await send('Runtime.evaluate', {
    expression: `
      const chatItem = document.querySelector('.mc-chat-item') || Array.from(document.querySelectorAll('button')).find(b => b.innerText.includes('TESTE'));
      if (chatItem) chatItem.click();
    `
  });
  await new Promise(r => setTimeout(r, 800));

  // Show topbar by triggering scroll or toggle
  await send('Runtime.evaluate', {
    expression: `
      window.dispatchEvent(new CustomEvent('scroll'));
      const scroller = document.querySelector('.mc-scroller');
      if (scroller) {
        scroller.scrollTop = 100;
        setTimeout(() => { scroller.scrollTop = 0; }, 100);
      }
      const showBtn = document.querySelector('button[aria-label*="barra"], button[aria-label*="bar"]');
      if (showBtn) showBtn.click();
    `
  });
  await new Promise(r => setTimeout(r, 800));

  const snap = await send('Page.captureScreenshot', { format: 'png' });
  fs.writeFileSync('mockup_02_chat_topbar.png', Buffer.from(snap.data, 'base64'));
  console.log('Saved mockup_02_chat_topbar.png');
  ws.close();
}

run().catch(console.error);
